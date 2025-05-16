use native_tls::Error as TlsError;
use std::net::TcpStream;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("URL parsing error: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Missing host in URL")]
    MissingHost,

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("TLS error: {0}")]
    TlsError(#[from] TlsError),

    #[error("Handshake error: {0}")]
    HandshakeError(#[from] native_tls::HandshakeError<TcpStream>),

    #[error("Response parse error: {0}")]
    ResponseParseError(String),
}

#[derive(Debug, Error)]
pub enum ResponseError {
    #[error("Response is empty")]
    EmptyResponse,

    #[error("Missing status code in response")]
    MissingStatusCode,

    #[error("Invalid status code in response")]
    InvalidStatusCode,

    #[error("Missing meta description in response")]
    MissingMetaDescription,

    #[error("Error parsing the body: {0}")]
    BodyParseError(#[from] std::io::Error),

    #[error("General response parsing error: {0}")]
    GeneralParseError(String),

    #[error(transparent)]
    Infallible(#[from] std::convert::Infallible),
}
