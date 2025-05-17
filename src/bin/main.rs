#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![deny(clippy::complexity)]
#![deny(clippy::style)]
#![deny(clippy::correctness)]
#![warn(clippy::unused_io_amount)]
#![warn(clippy::unnecessary_unwrap)]
#![warn(clippy::expect_used)]

use clap::Parser;
use once_cell::sync::Lazy;
use std::process::exit;

use env_logger::Env;
use log::{error, warn};
use std::io::{self};
use url::Url;

use gemini::client::Client;
use gemini::handlers::handle_request;
use gemini::models::Pager;

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
