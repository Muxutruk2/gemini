#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![deny(clippy::complexity)]
#![deny(clippy::style)]
#![deny(clippy::correctness)]
#![warn(clippy::unused_io_amount)]
#![warn(clippy::unnecessary_unwrap)]
#![warn(clippy::expect_used)]

use clap::{Parser, ValueEnum};
use crossterm::{
    cursor, execute,
    terminal::{Clear, ClearType},
};
use once_cell::sync::Lazy;
use rustyline::{self, error::ReadlineError, history::MemHistory, Config};
use std::process::{exit, Command, Stdio};

use colored::Colorize;

use env_logger::Env;
use log::{debug, error, info, trace, warn};
use native_tls::TlsConnector;
use std::io::{self, stdout, BufReader, Read, Write};
use std::net::TcpStream;
use url::{ParseError, Url};

use gemini::errors::{RequestError, ResponseError};

#[derive(Debug)]
enum StatusCode {
    Input,
    Success,
    Redirect,
    TemporaryFailure,
    PermanentFailure,
    ClientCertificateRequired,
    Unknown(u8),
}

impl From<u8> for StatusCode {
    fn from(code: u8) -> Self {
        match code {
            10..=19 => StatusCode::Input,
            20..=29 => StatusCode::Success,
            30..=39 => StatusCode::Redirect,
            40..=49 => StatusCode::TemporaryFailure,
            50..=59 => StatusCode::PermanentFailure,
            60..=69 => StatusCode::ClientCertificateRequired,
            _ => StatusCode::Unknown(code),
        }
    }
}

#[derive(Debug)]
struct Request {
    url: Url,
}

impl Request {
    fn new(url: Url) -> Self {
        info!("Creating new request for URL: {url}");
        Self { url }
    }

    fn send(&self) -> Result<Result<Response, ResponseError>, RequestError> {
        info!("Sending request to: {}", self.url);

        let connector = TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()?;

        let host = self
            .url
            .host_str()
            .ok_or(RequestError::MissingHost)?
            .to_string();

        let port = self.url.port().unwrap_or(1965);

        debug!("Connecting to {host} on port {port}");
        let stream = TcpStream::connect(format!("{host}:{port}"))?;
        let mut stream = connector.connect(&host, stream)?;

        let request = format!("gemini://{host}{}\r\n", self.url.path());
        debug!("Sending request: {request:?}");

        stream.write_all(request.as_bytes())?;
        stream.flush()?;
        info!("Request sent successfully");

        let mut reader = BufReader::new(stream);
        let mut string_response = String::new();

        reader.read_to_string(&mut string_response)?;

        trace!("Raw response received: {string_response:?}");

        Ok(Response::try_from(string_response.as_str()))
    }
}

impl TryFrom<&str> for Request {
    type Error = RequestError;

    fn try_from(url: &str) -> Result<Self, RequestError> {
        info!("Parsing URL: {url}");
        let url = Url::parse(url)?;
        Ok(Self::new(url))
    }
}

#[derive(Debug)]
struct Link {
    href: String,
    name: Option<String>,
}

enum LinkParseError {
    InvalidFormat,
}

impl TryFrom<&str> for Link {
    type Error = LinkParseError;

    fn try_from(line: &str) -> Result<Self, Self::Error> {
        // Strip leading whitespace and check if the line starts with "=>"
        let trimmed = line.trim_start();
        if !trimmed.starts_with("=>") {
            return Err(LinkParseError::InvalidFormat);
        }

        let trimmed = trimmed.trim_start_matches("=>").trim();

        let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();

        if let Some(url) = parts.first() {
            let name = parts.get(1).map(|s| (*s).to_string());
            Ok(Link {
                href: (*url).to_string(),
                name,
            })
        } else {
            Err(LinkParseError::InvalidFormat)
        }
    }
}

struct Response {
    status_code: StatusCode,
    meta_description: String,
    body: Option<String>,
    links: Vec<Link>,
}

impl TryFrom<&str> for Response {
    type Error = ResponseError;

    fn try_from(response_str: &str) -> Result<Self, ResponseError> {
        debug!("Parsing response string");

        let mut lines = response_str.lines();
        let first_line = lines.next().ok_or(ResponseError::EmptyResponse)?;

        let mut first_line_parts = first_line.splitn(2, ' ');
        let status_code = StatusCode::from(
            first_line_parts
                .next()
                .ok_or(ResponseError::MissingStatusCode)?
                .parse::<u8>()
                .map_err(|_| ResponseError::InvalidStatusCode)?,
        );

        let meta_description = first_line_parts
            .next()
            .ok_or(ResponseError::MissingMetaDescription)?
            .to_string();

        let body = lines.collect::<Vec<&str>>().join("\n");

        let links: Vec<Link> = body
            .lines()
            .filter_map(|line| {
                if line.starts_with("=>") {
                    Link::try_from(line).ok()
                } else {
                    None
                }
            })
            .collect();

        trace!(
            "Response parsed: status_code={status_code:?}, meta={meta_description}, body={body:?}, links={links:?}",
        );

        Ok(Self {
            status_code,
            meta_description,
            body: if body.is_empty() { None } else { Some(body) },
            links,
        })
    }
}

struct Client {
    current_url: Url,
    redirects: usize,
    max_redirects: usize,
    history: Vec<Url>,
    last_working_url: Option<Url>,
    pager: Pager,
}

impl Client {
    fn new(url: &Url, pager: Pager) -> Self {
        Self {
            current_url: url.clone(),
            redirects: 0,
            max_redirects: 5,
            history: vec![],
            last_working_url: None,
            pager,
        }
    }

    fn request(&mut self, url: Url) -> Result<Result<Response, ResponseError>, RequestError> {
        self.history.push(url.clone()); // Store URL in history
        self.current_url = url.clone();
        Request::new(url).send()
    }

    fn click_link(&mut self, link: &str) -> Result<Url, ParseError> {
        if let Ok(parsed_url) = Url::parse(link) {
            if parsed_url.scheme() == "gemini" {
                if parsed_url == self.current_url {
                    return self
                        .previous_url()
                        .cloned()
                        .ok_or(ParseError::RelativeUrlWithoutBase);
                }
                return Ok(parsed_url);
            }
            Command::new("xdg-open")
                .arg(parsed_url.as_str())
                .spawn()
                .ok();
            return Ok(self.current_url.clone());
        }

        let url = if link.starts_with("//") {
            Url::parse(&format!("{}:{}", self.current_url.scheme(), link))?
        } else if link.starts_with('/') {
            self.current_url.join(link)?
        } else {
            self.current_url.join(&format!("./{link}"))?
        };

        Ok(url)
    }

    fn previous_url(&self) -> Option<&Url> {
        self.history.last()
    }

    fn actual_previous_url(&self) -> Option<&Url> {
        if self.history.len() >= 2 {
            self.history.get(self.history.len() - 2)
        } else {
            None
        }
    }
}

fn exit_with_error(msg: &str) -> ! {
    error!("{msg}");
    exit(1);
}

static LOGGER: Lazy<()> = Lazy::new(|| {
    let env = Env::default().filter_or("RUST_LOG", "debug");
    env_logger::Builder::from_env(env)
        .format_timestamp(None)
        .init();
});

fn initialize_url(input: String) -> Url {
    let full_url = if input.starts_with("gemini://") {
        input
    } else {
        format!("gemini://{input}")
    };

    match Url::try_from(full_url.as_str()) {
        Ok(u) => u,
        Err(e) => exit_with_error(&format!("Could not parse URL: {e}")),
    }
}

fn main_loop(client: &mut Client, mut url: Url) -> io::Result<()> {
    loop {
        match handle_request(client, &url) {
            Some(new_url) => url = new_url,
            None => return Ok(()),
        }
    }
}

fn handle_request(client: &mut Client, url: &Url) -> Option<Url> {
    match client.request(url.clone()) {
        Ok(Ok(response)) => match response.status_code {
            StatusCode::Input => Some(url.clone()),
            StatusCode::Success => handle_success(client, &response, url),
            StatusCode::Redirect => handle_redirect(client, &response, url),
            StatusCode::TemporaryFailure
            | StatusCode::PermanentFailure
            | StatusCode::ClientCertificateRequired => {
                error!("{}", response.meta_description);
                None
            }
            StatusCode::Unknown(code) => exit_with_error(&format!("INVALID STATUS CODE {code}")),
        },
        Ok(Err(e)) => exit_with_error(&format!("Response Error: {e:?}")),
        Err(e) => exit_with_error(&format!("Response Error: {e:?}")),
    }
}

#[derive(ValueEnum, Debug, Clone, Copy)]
#[clap(rename_all = "lower")]
enum Pager {
    Less,
    More,
    Bat,
    Neovim,
}

#[derive(Parser, Debug)]
#[command(name = "gemini-client")]
#[command(about = "A simple Gemini protocol client", long_about = None)]
struct Cli {
    url: String,

    #[arg(long, value_enum, default_value_t = Pager::Less)]
    pager: Pager,
}

fn main() -> io::Result<()> {
    Lazy::force(&LOGGER);

    let cli = Cli::parse();
    let url = initialize_url(cli.url);
    let mut client = Client::new(&url, cli.pager);

    main_loop(&mut client, url)
}

fn handle_success(client: &mut Client, response: &Response, url: &Url) -> Option<Url> {
    debug!("Success!");
    client.last_working_url = Some(url.clone());
    client.redirects = 0;

    let mut pager = spawn_pager(client.pager).expect("Failed to spawn pager");

    if let Some(stdin) = pager.stdin.as_mut() {
        stdin
            .write_all(response.body.as_deref().unwrap_or("No content").as_bytes())
            .expect("Failed to write to pager stdin");
        writeln!(stdin, "\n").expect("Failed to write to pager"); // 2 new lines
        for (i, link) in response.links.iter().enumerate() {
            writeln!(
                stdin,
                "{}: {} ({})",
                i.to_string().blue(),
                link.name.as_deref().unwrap_or("").bright_white(),
                link.href.blue()
            )
            .expect("Failed to write links to pager stdin");
        }
    }

    pager.wait().expect("Error waiting for pager");

    let mut stdout = stdout();

    execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 1)).unwrap();

    handle_user_input(client, response, url)
}

fn spawn_pager(pager: Pager) -> std::io::Result<std::process::Child> {
    let mut cmd = match pager {
        Pager::Less => {
            let mut c = Command::new("less");
            c.arg("-R");
            c
        }
        Pager::More => Command::new("more"),
        Pager::Bat => {
            let mut c = Command::new("bat");
            c.arg("--paging=always");
            c.arg("--decorations=never");
            c
        }
        Pager::Neovim => {
            let mut c = Command::new("nvim");
            c.arg("+Man!");
            c
        }
    };

    cmd.stdin(Stdio::piped()).spawn()
}

fn handle_redirect(client: &mut Client, response: &Response, _url: &Url) -> Option<Url> {
    info!("Redirecting to {}", response.meta_description);
    client.redirects += 1;

    if client.redirects >= client.max_redirects {
        error!("Too many redirects!");
        return client.last_working_url.clone();
    }

    match client.click_link(&response.meta_description) {
        Ok(new_url) => Some(new_url),
        Err(e) => {
            error!("Error parsing redirect: {e}");
            None
        }
    }
}

fn handle_user_input(client: &mut Client, response: &Response, url: &Url) -> Option<Url> {
    let mut rl =
        rustyline::Editor::<(), MemHistory>::with_history(Config::default(), MemHistory::default())
            .expect("Failed creating editor");

    let prompt = "Select a link by number or type a new URL ([q]uit [b]ack [r]eload): "
        .yellow()
        .to_string();

    let input_result = rl.readline(&prompt);

    match input_result {
        Ok(_) => {}
        Err(ref e) => match e {
            ReadlineError::Interrupted | ReadlineError::Eof => {
                println!("Goodbye!");
                std::process::exit(1);
            }
            _ => {
                error!("Error on rustyline: {e}");
            }
        },
    }

    let input = input_result.unwrap();

    match input.as_str() {
        "q" => {
            println!("Goodbye!");
            None
        }
        "b" => client.actual_previous_url().cloned(),
        "r" => client.previous_url().cloned(),
        _ if input
            .parse::<usize>()
            .ok()
            .and_then(|index| response.links.get(index))
            .and_then(|link| client.click_link(&link.href).ok())
            .is_some() =>
        {
            client
                .click_link(&response.links[input.parse::<usize>().unwrap()].href)
                .ok()
        }
        _ if Url::parse(&input).is_ok() => Some(Url::parse(&input).unwrap()),
        _ => {
            println!("Invalid input. Please try again.");
            Some(url.clone())
        }
    }
}
