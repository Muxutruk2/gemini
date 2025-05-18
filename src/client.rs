use std::process::Command;
use url::{ParseError, Url};

use crate::errors::{RequestError, ResponseError};
use crate::handlers::get_edit_prompt;
use crate::models::{Pager, Request, Response};

pub struct Client {
    pub current_url: Url,
    pub redirects: usize,
    pub max_redirects: usize,
    pub history: Vec<Url>,
    pub last_working_url: Option<Url>,
    pub pager: Pager,
}

impl Client {
    pub fn new(url: &Url, pager: Pager) -> Self {
        Self {
            current_url: url.clone(),
            redirects: 0,
            max_redirects: 5,
            history: vec![],
            last_working_url: None,
            pager,
        }
    }

    pub fn request(&mut self, url: Url) -> Result<Result<Response, ResponseError>, RequestError> {
        self.history.push(url.clone()); // Store URL in history
        self.current_url = url.clone();
        Request::new(url).send()
    }

    pub fn click_link(&mut self, link: &str) -> Result<Url, ParseError> {
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

    pub fn previous_url(&self) -> Option<&Url> {
        self.history.last()
    }

    pub fn actual_previous_url(&self) -> Option<&Url> {
        if self.history.len() >= 2 {
            self.history.get(self.history.len() - 2)
        } else {
            None
        }
    }

    pub fn edit_url(&mut self) -> Option<Url> {
        get_edit_prompt(self.current_url.as_str())
            .map(|a| Url::parse(&a).map_err(|_| None::<Url>).unwrap())
    }
}
