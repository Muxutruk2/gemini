use colored::Colorize;
use crossterm::{
    cursor, execute,
    terminal::{Clear, ClearType},
};
use log::{debug, error, info};
use rpassword::read_password;
use rustyline::{self, history::MemHistory, Config};
use std::fs;
use std::io::{self, stdout, Write};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;
use url::Url;

use crate::client::Client;
use crate::models::{Pager, Response, StatusCode};

pub fn handle_request(client: &mut Client, url: &Url) -> Option<Url> {
    match client.request(url.clone()) {
        Ok(Ok(response)) => match response.status_code {
            StatusCode::Input => handle_input(client, &response, url),
            StatusCode::Success => handle_success(client, &response, url),
            StatusCode::Redirect => handle_redirect(client, &response, url),
            StatusCode::TemporaryFailure
            | StatusCode::PermanentFailure
            | StatusCode::ClientCertificateRequired => {
                error!("{}", response.meta_description);
                None
            }
            StatusCode::Unknown(code) => {
                error!("INVALID STATUS CODE {code}");
                None
            }
        },
        Ok(Err(e)) => {
            error!("Response Error: {e:?}");
            None
        }
        Err(e) => {
            error!("Response Error: {e:?}");
            None
        }
    }
}

pub fn handle_success(client: &mut Client, response: &Response, url: &Url) -> Option<Url> {
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

    get_client_prompt(client, response, url)
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

pub fn handle_redirect(client: &mut Client, response: &Response, _url: &Url) -> Option<Url> {
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

pub fn handle_input(client: &mut Client, response: &Response, _url: &Url) -> Option<Url> {
    info!("Page asks for user input");

    let input = match response.status_code_num {
        10 => get_user_input(&response.meta_description),
        _ => get_secure_user_input(&response.meta_description),
    };

    let mut new_url = client.previous_url().unwrap().clone();

    new_url.set_query(input.as_deref());

    Some(new_url)
}

fn get_client_prompt(client: &mut Client, response: &Response, url: &Url) -> Option<Url> {
    let prompt = "Select a link by number or type a new URL ([q]uit [b]ack [r]eload [e]dit): ";

    let input = get_user_input(prompt);

    match input {
        Some(_) => {}
        None => {
            std::process::exit(1);
        }
    };

    let input = input.unwrap();

    match input.as_str() {
        "q" => {
            println!("Goodbye!");
            None
        }
        "b" => client.actual_previous_url().cloned(),
        "r" => client.previous_url().cloned(),
        "e" => client.edit_url(),
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

fn get_user_input(prompt: &str) -> Option<String> {
    let mut rl =
        rustyline::Editor::<(), MemHistory>::with_history(Config::default(), MemHistory::default())
            .expect("Failed creating editor");

    let prompt = prompt.yellow().to_string();

    let input_result = rl.readline(&prompt);

    input_result.map(Some).unwrap_or_else(|e| {
        error!("Error on getting input: {e}");
        None
    })
}

fn get_secure_user_input(prompt: &str) -> Option<String> {
    print!("{}", prompt.yellow());

    io::stdout().flush().ok()?;

    match read_password() {
        Ok(password) => Some(password),
        Err(e) => {
            error!("Error getting password input: {e}");
            None
        }
    }
}

pub fn get_edit_prompt(text: &str) -> Option<String> {
    let mut tmpfile = NamedTempFile::new().ok()?;

    write!(tmpfile, "{text}").ok()?;

    let path = tmpfile.path().to_owned();

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());

    let status = Command::new(editor).arg(&path).status().ok()?;

    if status.success() {
        fs::read_to_string(path).ok()
    } else {
        None
    }
}
