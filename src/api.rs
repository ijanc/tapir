use std::io::{BufReader, Read};
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::sse::SseReader;
use crate::types::{ApiError, Request};

const MAX_ATTEMPTS: u32 = 3;
const HTTP_TIMEOUT: u64 = 60;

pub fn send_stream(
    config: &Config,
    request: &Request<'_>,
) -> Result<SseReader> {
    let body = serde_json::to_string(request)?;

    for attempt in 1..=MAX_ATTEMPTS {
        match try_send(config, &body) {
            Ok(reader) => return Ok(reader),
            Err(ref e) if attempt < MAX_ATTEMPTS && is_retryable(e) => {
                let delay = retry_delay(e, attempt);
                eprintln!(
                    "* retry {attempt}/{MAX_ATTEMPTS} \
                     in {delay}s ({e})"
                );
                thread::sleep(Duration::from_secs(delay));
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!()
}

fn try_send(config: &Config, body: &str) -> Result<SseReader> {
    let mut response = minreq::post(&config.api_url)
        .with_header("x-api-key", &config.api_key)
        .with_header("anthropic-version", "2023-06-01")
        .with_header("anthropic-beta", "prompt-caching-2024-07-31")
        .with_header("content-type", "application/json")
        .with_body(body)
        .with_timeout(HTTP_TIMEOUT)
        .send_lazy()
        .map_err(|e| Error::Http(e.to_string()))?;

    let status = response.status_code as u16;

    if status != 200 {
        let retry_after = response
            .headers
            .get("retry-after")
            .and_then(|v| v.parse::<u64>().ok());

        let mut text = String::new();
        response
            .read_to_string(&mut text)
            .map_err(|e| Error::Http(e.to_string()))?;

        let api_err: ApiError =
            serde_json::from_str(&text).unwrap_or(ApiError {
                error: crate::types::ApiErrorDetail {
                    kind: "unknown".to_string(),
                    message: text,
                },
            });
        return Err(Error::Api {
            status,
            message: api_err.error.message,
            retry_after,
        });
    }

    let reader = BufReader::new(response);
    Ok(SseReader::new(Box::new(reader)))
}

fn is_retryable(err: &Error) -> bool {
    match err {
        Error::Http(_) => true,
        Error::Api { status, .. } => {
            matches!(status, 429 | 500 | 502 | 503 | 529)
        }
        _ => false,
    }
}

fn retry_delay(err: &Error, attempt: u32) -> u64 {
    if let Error::Api {
        retry_after: Some(secs),
        ..
    } = err
    {
        return *secs;
    }
    1u64 << (attempt - 1) // 1, 2, 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_http_error() {
        assert!(is_retryable(&Error::Http("timeout".into())));
    }

    #[test]
    fn test_is_retryable_transient_api_errors() {
        for status in [429, 500, 502, 503, 529] {
            let err = Error::Api {
                status,
                message: "error".into(),
                retry_after: None,
            };
            assert!(is_retryable(&err), "status {status} should be retryable",);
        }
    }

    #[test]
    fn test_not_retryable_client_errors() {
        for status in [400, 401, 403, 404] {
            let err = Error::Api {
                status,
                message: "error".into(),
                retry_after: None,
            };
            assert!(
                !is_retryable(&err),
                "status {status} should not be retryable",
            );
        }
    }

    #[test]
    fn test_not_retryable_other_errors() {
        assert!(!is_retryable(&Error::NoApiKey));
        assert!(!is_retryable(&Error::Json("bad".into())));
    }

    #[test]
    fn test_retry_delay_exponential() {
        let err = Error::Http("timeout".into());
        assert_eq!(retry_delay(&err, 1), 1);
        assert_eq!(retry_delay(&err, 2), 2);
        assert_eq!(retry_delay(&err, 3), 4);
    }

    #[test]
    fn test_retry_delay_respects_retry_after() {
        let err = Error::Api {
            status: 429,
            message: "rate limited".into(),
            retry_after: Some(30),
        };
        assert_eq!(retry_delay(&err, 1), 30);
    }
}
