//! OpenAI-compatible chat-completion client for pinned teachers.
//!
//! Endpoint, model id, and credential env-var name come from the pin entry.
//! The credential value is held only in memory and is never logged or written
//! into an error, record, or manifest.

use std::time::Duration;

use serde_json::{Value, json};

use crate::teacher::prompt::count_tokens;
use crate::teacher::{PreparedTeacher, TeacherError};

/// One teacher completion request.
#[derive(Debug, Clone)]
pub struct TeacherRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// The teacher's raw text output.
#[derive(Debug, Clone)]
pub struct TeacherCompletion {
    pub content: String,
    /// The `model` field the endpoint reported, when it did. Pinned variants
    /// carry dated names while providers answer with an alias; capturing it
    /// keeps the draft's rights envelope honest about what served the draft.
    pub served_model: Option<String>,
}

/// A teacher transport. Tests supply recorded responses; production uses
/// [`HttpTeacherClient`].
pub trait TeacherClient {
    fn complete(&self, request: &TeacherRequest) -> Result<TeacherCompletion, TeacherError>;
}

/// HTTP client for OpenAI-compatible endpoints.
pub struct HttpTeacherClient {
    client: reqwest::blocking::Client,
    endpoint: String,
    api_key: String,
}

impl HttpTeacherClient {
    /// Build a client for a prepared teacher. The credential is moved into the
    /// client and never exposed.
    pub fn new(prepared: &PreparedTeacher) -> Result<Self, TeacherError> {
        Self::from_endpoint(
            prepared.pin().endpoint.clone(),
            prepared.credentials().to_owned(),
        )
    }

    /// Build a client from an endpoint and credential directly.
    ///
    /// Production uses [`HttpTeacherClient::new`]; this constructor exists for
    /// transport tests that point at a local server. The credential is still
    /// held only in memory.
    pub fn from_endpoint(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, TeacherError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| TeacherError::Transport(format!("cannot build HTTP client: {e}")))?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            api_key: api_key.into(),
        })
    }
}

impl TeacherClient for HttpTeacherClient {
    fn complete(&self, request: &TeacherRequest) -> Result<TeacherCompletion, TeacherError> {
        let body = json!({
            "model": request.model,
            "messages": [
                {"role": "system", "content": request.system},
                {"role": "user", "content": request.user}
            ],
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
            "response_format": {"type": "json_object"}
        });
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| TeacherError::Transport(format!("request failed: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            // Deliberately omit the body: it can echo request metadata and is
            // not needed to diagnose an HTTP-level failure.
            return Err(TeacherError::Transport(format!(
                "endpoint returned HTTP {}",
                status.as_u16()
            )));
        }
        let value: Value = response
            .json()
            .map_err(|e| TeacherError::Parse(format!("invalid response JSON: {e}")))?;
        parse_chat_completion(&value)
    }
}

/// Extract `choices[0].message.content` from an OpenAI-compatible response.
pub fn parse_chat_completion(value: &Value) -> Result<TeacherCompletion, TeacherError> {
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or(TeacherError::MissingContent)?;
    let served_model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(TeacherCompletion {
        content: content.to_owned(),
        served_model,
    })
}

/// Token count of a completion, for the output budget check.
pub fn completion_tokens(content: &str) -> u32 {
    count_tokens(content)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    fn serve_once(status: u16, body: String) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Drain the full request: reqwest sends headers plus the JSON
            // body, which exceeds a single 8 KiB read. A truncated request
            // makes the client reset the connection mid-write.
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            let mut chunk = [0u8; 8192];
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
            }
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (format!("http://{address}/chat/completions"), handle)
    }

    fn request() -> TeacherRequest {
        TeacherRequest {
            model: "deepseek-flash".to_owned(),
            system: "system".to_owned(),
            user: "user".to_owned(),
            max_tokens: 64,
            temperature: 0.2,
        }
    }

    #[test]
    fn http_client_posts_and_parses_a_completion() {
        let body = json!({
            "choices": [{"message": {"content": "{\"ok\":true}"}}]
        })
        .to_string();
        let (endpoint, handle) = serve_once(200, body);
        let client = HttpTeacherClient::from_endpoint(endpoint, "test-key").unwrap();
        let completion = client.complete(&request()).unwrap();
        assert_eq!(completion.content, "{\"ok\":true}");
        handle.join().unwrap();
    }

    #[test]
    fn http_client_reports_status_without_the_body() {
        let (endpoint, handle) = serve_once(500, "sensitive-body".to_owned());
        let client = HttpTeacherClient::from_endpoint(endpoint, "test-key").unwrap();
        let error = client.complete(&request()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("HTTP 500"));
        assert!(!message.contains("sensitive-body"));
        assert!(!message.contains("test-key"));
        handle.join().unwrap();
    }
}
