use clap::ValueEnum;
use log::{debug, info, trace};
use native_tls::TlsConnector;
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use url::Url;

use crate::errors::{RequestError, ResponseError};

#[derive(ValueEnum, Debug, Clone, Copy)]
#[clap(rename_all = "lower")]
pub enum Pager {
    Less,
    More,
    Bat,
    Neovim,
}

#[derive(Debug)]
pub enum StatusCode {
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
pub struct Request {
    url: Url,
}

impl Request {
    pub fn new(url: Url) -> Self {
        info!("Creating new request for URL: {url}");
        Self { url }
    }

    pub fn send(&self) -> Result<Result<Response, ResponseError>, RequestError> {
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
pub struct Link {
    pub href: String,
    pub name: Option<String>,
}

pub enum LinkParseError {
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

pub struct Response {
    pub status_code: StatusCode,
    pub status_code_num: u8,
    pub meta_description: String,
    pub body: Option<String>,
    pub links: Vec<Link>,
}

impl TryFrom<&str> for Response {
    type Error = ResponseError;

    fn try_from(response_str: &str) -> Result<Self, ResponseError> {
        debug!("Parsing response string");

        let mut count: u32 = 0;

        let mut lines = response_str.lines();
        let first_line = lines.next().ok_or(ResponseError::EmptyResponse)?;

        let mut first_line_parts = first_line.splitn(2, ' ');

        let status_code_num = first_line_parts
            .next()
            .ok_or(ResponseError::MissingStatusCode)?
            .parse::<u8>()
            .map_err(|_| ResponseError::InvalidStatusCode)?;

        let status_code = StatusCode::from(status_code_num);

        let meta_description = first_line_parts
            .next()
            .ok_or(ResponseError::MissingMetaDescription)?
            .to_string();

        let body = lines
            .clone()
            .map(|line| {
                if line.trim_start().starts_with("=>") {
                    let result = format!("({}) {}", count, line.trim_start());
                    count += 1;
                    result
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let links: Vec<Link> = lines
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
            status_code_num,
            meta_description,
            body: if body.is_empty() { None } else { Some(body) },
            links,
        })
    }
}
