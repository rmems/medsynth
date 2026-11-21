//! Teacher fulfillment via recorded response files.
//!
//! The pipeline never calls a teacher API itself. Each request is written to
//! the fulfillment directory as `<key>.request.json` — an OpenAI-compatible
//! chat-completion body an external harness can dispatch however it likes —
//! and the harness drops the teacher's raw response at `<key>.response.json`.
//! The key is a hash of the exact request-file bytes, so responses cannot be
//! mixed up across cells, attempts, pins, or runs.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::hash::sha256_str;
use crate::teacher::TeacherError;
use crate::teacher::prompt::count_tokens;

/// One teacher completion request.
#[derive(Debug, Clone)]
pub struct TeacherRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// The attempt ordinal, sent as the API `seed` so each attempt of a cell
    /// asks for a distinct sample and fulfills under its own key.
    pub seed: u32,
    /// The pin driving the request, carried in `metadata` so a revised pin
    /// cannot silently reuse a previous variant's response file.
    pub pin_id: String,
    /// The pinned variant, for the same reason.
    pub pin_variant: String,
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

/// A teacher fulfillment source. Production uses [`RecordedTeacherClient`];
/// tests supply recorded responses directly.
pub trait TeacherClient {
    fn complete(&self, request: &TeacherRequest) -> Result<TeacherCompletion, TeacherError>;
}

/// Serialize `request` as the OpenAI-compatible chat-completion body that
/// identifies the fulfillment.
fn request_body(request: &TeacherRequest) -> Value {
    json!({
        "model": request.model,
        "messages": [
            {"role": "system", "content": request.system},
            {"role": "user", "content": request.user}
        ],
        "temperature": request.temperature,
        "max_tokens": request.max_tokens,
        "seed": request.seed,
        "metadata": {"pin_id": request.pin_id, "variant": request.pin_variant},
        "response_format": {"type": "json_object"}
    })
}

/// The exact bytes written to `<key>.request.json`.
fn request_body_text(request: &TeacherRequest) -> String {
    serde_json::to_string_pretty(&request_body(request)).unwrap()
}

/// The filename stem identifying a fulfillment: the request file's full
/// SHA-256, so a harness that hashes the file bytes derives the same key and
/// distinct bodies can never collide.
pub fn fulfillment_key(request: &TeacherRequest) -> String {
    sha256_str(&request_body_text(request))
}

/// Fulfills teacher requests from a directory of recorded responses.
///
/// On each request the client writes `<dir>/<key>.request.json` (creating the
/// directory if needed) so the external harness knows exactly what to
/// dispatch, then reads `<dir>/<key>.response.json`. A missing response fails
/// the attempt; a malformed one fails parsing.
pub struct RecordedTeacherClient {
    dir: PathBuf,
}

impl RecordedTeacherClient {
    /// `dir` is the fulfillment directory shared with the external harness.
    pub fn new(dir: PathBuf) -> Result<Self, TeacherError> {
        std::fs::create_dir_all(&dir).map_err(|e| {
            TeacherError::Transport(format!(
                "cannot create fulfillment dir {}: {e}",
                dir.display()
            ))
        })?;
        Ok(Self { dir })
    }

    fn paths(&self, request: &TeacherRequest) -> (PathBuf, PathBuf) {
        let key = fulfillment_key(request);
        (
            self.dir.join(format!("{key}.request.json")),
            self.dir.join(format!("{key}.response.json")),
        )
    }
}

impl TeacherClient for RecordedTeacherClient {
    fn complete(&self, request: &TeacherRequest) -> Result<TeacherCompletion, TeacherError> {
        let (request_path, response_path) = self.paths(request);
        // Stage then rename: a pre-existing symlink at the request path is
        // replaced by the rename, never followed to truncate its target.
        let tmp_path = request_path.with_extension("tmp");
        std::fs::write(&tmp_path, request_body_text(request))
            .and_then(|()| std::fs::rename(&tmp_path, &request_path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                TeacherError::Transport(format!(
                    "cannot write request file {}: {e}",
                    request_path.display()
                ))
            })?;
        let text = match std::fs::read_to_string(&response_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(TeacherError::Transport(format!(
                    "no recorded response at {}; dispatch {} externally and drop the teacher's raw chat-completion JSON there",
                    response_path.display(),
                    request_path.display()
                )));
            }
            Err(e) => {
                return Err(TeacherError::Transport(format!(
                    "cannot read response file {}: {e}",
                    response_path.display()
                )));
            }
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            TeacherError::Parse(format!(
                "invalid response JSON in {}: {e}",
                response_path.display()
            ))
        })?;
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
    use super::*;

    struct FulfillmentDir {
        path: PathBuf,
    }

    impl FulfillmentDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "medsynth-fulfillment-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for FulfillmentDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn request() -> TeacherRequest {
        TeacherRequest {
            model: "deepseek-flash".to_owned(),
            system: "system".to_owned(),
            user: "user".to_owned(),
            max_tokens: 64,
            temperature: 0.2,
            seed: 0,
            pin_id: "deepseek-v4.1-flash".to_owned(),
            pin_variant: "DeepSeek-V4.1-Flash-1017".to_owned(),
        }
    }

    #[test]
    fn recorded_client_reads_the_response_file() {
        let dir = FulfillmentDir::new("ok");
        let client = RecordedTeacherClient::new(dir.path.clone()).unwrap();
        // First call emits the request file and reports the missing response.
        let error = client.complete(&request()).unwrap_err();
        assert!(matches!(error, TeacherError::Transport(_)), "{error}");
        let request_path = dir
            .path
            .join(format!("{}.request.json", fulfillment_key(&request())));
        assert!(request_path.exists());
        let body: Value =
            serde_json::from_str(&std::fs::read_to_string(&request_path).unwrap()).unwrap();
        assert_eq!(body["model"], "deepseek-flash");
        assert_eq!(body["response_format"]["type"], "json_object");
        // The key a harness derives by hashing the request file itself.
        assert_eq!(
            fulfillment_key(&request()),
            sha256_str(&std::fs::read_to_string(&request_path).unwrap())
        );
        // Harness fulfills: same stem, `.response.json`.
        let response_path = dir
            .path
            .join(format!("{}.response.json", fulfillment_key(&request())));
        std::fs::write(
            &response_path,
            json!({
                "model": "deepseek-flash-1017",
                "choices": [{"message": {"content": "{\"ok\":true}"}}]
            })
            .to_string(),
        )
        .unwrap();
        let completion = client.complete(&request()).unwrap();
        assert_eq!(
            (
                completion.content.as_str(),
                completion.served_model.as_deref()
            ),
            ("{\"ok\":true}", Some("deepseek-flash-1017"))
        );
    }

    #[test]
    fn recorded_client_classifies_a_missing_response_as_transport() {
        let dir = FulfillmentDir::new("missing");
        let client = RecordedTeacherClient::new(dir.path.clone()).unwrap();
        let error = client.complete(&request()).unwrap_err();
        assert!(matches!(error, TeacherError::Transport(_)), "{error}");
        assert!(error.to_string().contains("no recorded response"));
    }

    #[test]
    fn fulfillment_key_changes_with_attempt_seed_and_pin() {
        let base = fulfillment_key(&request());
        let mut other = request();
        other.seed = 1;
        assert_ne!(
            base,
            fulfillment_key(&other),
            "each attempt gets its own file"
        );
        let mut other = request();
        other.pin_variant = "DeepSeek-V4.1-Flash-1024".to_owned();
        assert_ne!(
            base,
            fulfillment_key(&other),
            "a revised pin cannot reuse the old response"
        );
    }

    #[test]
    fn recorded_client_classifies_malformed_json_as_parse() {
        let dir = FulfillmentDir::new("malformed");
        let client = RecordedTeacherClient::new(dir.path.clone()).unwrap();
        let _ = client.complete(&request());
        let response_path = dir
            .path
            .join(format!("{}.response.json", fulfillment_key(&request())));
        std::fs::write(&response_path, "not json").unwrap();
        let error = client.complete(&request()).unwrap_err();
        assert!(matches!(error, TeacherError::Parse(_)), "{error}");
    }
}
