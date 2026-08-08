use std::error::Error;
use std::fmt;
use std::time::Duration;

/// Reusable synchronous JSON HTTP client for Stainless applications.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    api_token: String,
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
    /// Creates a connection-pooling client. `api_token` may be empty for
    /// public endpoints.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not an HTTP(S) origin.
    pub fn new(base_url: &str, api_token: &str, timeout_ms: u64) -> Result<Self, ClientError> {
        let base_url = base_url.trim_end_matches('/');
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(ClientError::new(
                "the HTTP client base URL must start with http:// or https://",
            ));
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(timeout_ms.max(1))))
            .http_status_as_error(false)
            .build();
        Ok(Self {
            agent: config.into(),
            base_url: base_url.to_owned(),
            api_token: api_token.to_owned(),
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
        if !self.api_token.is_empty() {
            request = request.header("x-api-token", &self.api_token);
        }
        let response = request
            .call()
            .map_err(|error| ClientError::new(format!("GET {path} failed: {error}")))?;
        read_response("GET", path, response)
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
        let mut request = self
            .agent
            .post(url)
            .header("content-type", "application/json");
        if !self.api_token.is_empty() {
            request = request.header("x-api-token", &self.api_token);
        }
        let response = request
            .send(body)
            .map_err(|error| ClientError::new(format!("POST {path} failed: {error}")))?;
        read_response("POST", path, response)
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

fn read_response(
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
