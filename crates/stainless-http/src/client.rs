use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use ureq::http::{HeaderMap, HeaderName, HeaderValue, header::CONTENT_TYPE};

/// Reusable synchronous HTTP client for Stainless applications.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    headers: HeaderMap,
}

/// URL, transport, HTTP status, or response-body failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientError {
    message: String,
}

impl ClientError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ClientError {}

impl Client {
    /// Creates a connection-pooling client with reusable request headers.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not an HTTP(S) origin or a header
    /// name or value is invalid.
    pub fn new(
        base_url: &str,
        timeout_ms: u64,
        headers: &BTreeMap<String, String>,
    ) -> Result<Self, ClientError> {
        let base_url = base_url.trim_end_matches('/');
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(ClientError::new(
                "the HTTP client base URL must start with http:// or https://",
            ));
        }
        let mut parsed_headers = HeaderMap::new();
        for (name, value) in headers {
            let parsed_name = name.parse::<HeaderName>().map_err(|_| {
                ClientError::new(format!("invalid HTTP request header name `{name}`"))
            })?;
            let parsed_value = value.parse::<HeaderValue>().map_err(|_| {
                ClientError::new(format!("invalid value for HTTP request header `{name}`"))
            })?;
            parsed_headers.insert(parsed_name, parsed_value);
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(timeout_ms.max(1))))
            .http_status_as_error(false)
            .build();
        Ok(Self {
            agent: config.into(),
            base_url: base_url.to_owned(),
            headers: parsed_headers,
        })
    }

    /// Sends a GET request to one absolute path and returns its UTF-8 body.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success
    /// status codes, or unreadable response bodies.
    pub fn get(&self, path: &str) -> Result<String, ClientError> {
        let url = self.url(path)?;
        let mut request = self.agent.get(url);
        if let Some(headers) = request.headers_mut() {
            headers.extend(self.headers.clone());
        }
        let response = request
            .call()
            .map_err(|error| ClientError::new(format!("GET {path} failed: {error}")))?;
        read_text_response("GET", path, response)
    }

    /// Sends a GET request to one absolute path and returns its binary body.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success
    /// status codes, or unreadable response bodies.
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        let url = self.url(path)?;
        let mut request = self.agent.get(url);
        if let Some(headers) = request.headers_mut() {
            headers.extend(self.headers.clone());
        }
        let response = request
            .call()
            .map_err(|error| ClientError::new(format!("GET {path} failed: {error}")))?;
        read_binary_response("GET", path, response)
    }

    /// Sends a JSON POST request to one absolute path and returns its UTF-8
    /// response body.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success
    /// status codes, or unreadable response bodies.
    pub fn post_json(&self, path: &str, body: &str) -> Result<String, ClientError> {
        let url = self.url(path)?;
        let mut request = self.agent.post(url);
        if let Some(headers) = request.headers_mut() {
            headers.extend(self.headers.clone());
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        let response = request
            .send(body)
            .map_err(|error| ClientError::new(format!("POST {path} failed: {error}")))?;
        read_text_response("POST", path, response)
    }

    /// Sends a binary POST request to one absolute path and returns its binary
    /// response body. The request content type is `application/octet-stream`.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths, transport failures, non-success
    /// status codes, or unreadable response bodies.
    pub fn post_bytes(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, ClientError> {
        let url = self.url(path)?;
        let mut request = self.agent.post(url);
        if let Some(headers) = request.headers_mut() {
            headers.extend(self.headers.clone());
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
        }
        let response = request
            .send(body)
            .map_err(|error| ClientError::new(format!("POST {path} failed: {error}")))?;
        read_binary_response("POST", path, response)
    }

    fn url(&self, path: &str) -> Result<String, ClientError> {
        if !path.starts_with('/') || path.starts_with("//") {
            return Err(ClientError::new(
                "HTTP client paths must start with exactly one slash",
            ));
        }
        Ok(format!("{}{}", self.base_url, path))
    }
}

fn read_text_response(
    method: &str,
    path: &str,
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<String, ClientError> {
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| ClientError::new(format!("{method} {path} body failed: {error}")))?;
    if !status.is_success() {
        let detail = if body.is_empty() {
            status.to_string()
        } else {
            format!("{status}: {body}")
        };
        return Err(ClientError::new(format!(
            "{method} {path} returned {detail}"
        )));
    }
    Ok(body)
}

fn read_binary_response(
    method: &str,
    path: &str,
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<Vec<u8>, ClientError> {
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| ClientError::new(format!("{method} {path} body failed: {error}")))?;
    if !status.is_success() {
        let detail = if body.is_empty() {
            status.to_string()
        } else if let Ok(text) = std::str::from_utf8(&body) {
            format!("{status}: {text}")
        } else {
            format!("{status}: {} binary response bytes", body.len())
        };
        return Err(ClientError::new(format!(
            "{method} {path} returned {detail}"
        )));
    }
    Ok(body)
}
