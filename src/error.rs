use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    NoApiKey,
    Http(String),
    Api {
        status: u16,
        message: String,
        retry_after: Option<u64>,
    },
    Json(String),
    Tool {
        name: String,
        message: String,
    },
    Io(io::Error),
    Security(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoApiKey => {
                write!(f, "ANTHROPIC_API_KEY not set")
            }
            Error::Http(msg) => write!(f, "HTTP error: {msg}"),
            Error::Api {
                status, message, ..
            } => {
                write!(f, "API error ({status}): {message}")
            }
            Error::Json(msg) => write!(f, "JSON error: {msg}"),
            Error::Tool { name, message } => {
                write!(f, "tool {name}: {message}")
            }
            Error::Io(err) => write!(f, "I/O error: {err}"),
            Error::Security(msg) => {
                write!(f, "security: {msg}")
            }
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Json(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_display() {
        let err = Error::Api {
            status: 429,
            message: "rate limited".to_string(),
            retry_after: Some(30),
        };
        assert_eq!(err.to_string(), "API error (429): rate limited",);
    }

    #[test]
    fn test_api_error_without_retry_after() {
        let err = Error::Api {
            status: 500,
            message: "internal".to_string(),
            retry_after: None,
        };
        assert_eq!(err.to_string(), "API error (500): internal",);
    }
}
