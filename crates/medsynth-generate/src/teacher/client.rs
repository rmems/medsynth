//! Teacher fulfillment via recorded response files.
//!
//! The pipeline never calls a teacher API itself. Each request is written to
//! the fulfillment directory as `<key>.request.json` — an OpenAI-compatible
//! chat-completion body an external harness can dispatch however it likes —
//! and the harness drops the teacher's raw response at `<key>.response.json`.
//! Hermes routes also use `<key>.session.json` plus a hash-binding
//! `<key>.session.txt`, so replacement responses cannot reuse stale sessions.
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
    /// The batch seed, carried in `metadata` so two batches that differ only
    /// in `--seed` cannot share a fulfillment key — no u64 packing is
    /// injective, so the full seed travels as its own field.
    pub batch_seed: u64,
    /// The pin driving the request, carried in `metadata` so a revised pin
    /// cannot silently reuse a previous variant's response file.
    pub pin_id: String,
    /// The pinned variant, for the same reason.
    pub pin_variant: String,
    /// Ask a gateway route to include machine-readable vendor routing
    /// metadata in its response body.
    pub include_routing_metadata: bool,
    /// Whether this pin executes through Hermes and therefore uses session
    /// provenance. Direct routes ignore session sidecars entirely.
    pub requires_session_record: bool,
}

/// The teacher's raw text output.
#[derive(Debug, Clone)]
pub struct TeacherCompletion {
    pub content: String,
    /// The `model` field the endpoint reported, when it did. Pinned variants
    /// carry dated names while providers answer with an alias; capturing it
    /// keeps the draft's rights envelope honest about what served the draft.
    pub served_model: Option<String>,
    /// The execution vendor reported by a gateway (`routing.vendor_used` or
    /// an equivalent top-level vendor field), when present.
    pub served_vendor: Option<String>,
    /// Hermes session-record sidecar backing this completion, when supplied.
    pub session_file: Option<String>,
    /// SHA-256 of the exact session-record sidecar text.
    pub session_sha256: Option<String>,
}

/// A teacher fulfillment source. Production uses [`RecordedTeacherClient`];
/// tests supply recorded responses directly.
pub trait TeacherClient {
    fn complete(&self, request: &TeacherRequest) -> Result<TeacherCompletion, TeacherError>;
}

/// Serialize `request` as the OpenAI-compatible chat-completion body that
/// identifies the fulfillment.
fn request_body(request: &TeacherRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "messages": [
            {"role": "system", "content": request.system},
            {"role": "user", "content": request.user}
        ],
        "temperature": request.temperature,
        "max_tokens": request.max_tokens,
        "seed": request.seed,
        "metadata": {
            "pin_id": request.pin_id,
            "variant": request.pin_variant,
            "batch_seed": request.batch_seed
        },
        "response_format": {"type": "json_object"}
    });
    if request.include_routing_metadata {
        body["include_routing_metadata"] = json!(true);
    }
    body
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

    fn paths(&self, request: &TeacherRequest) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
        let key = fulfillment_key(request);
        (
            self.dir.join(format!("{key}.request.json")),
            self.dir.join(format!("{key}.response.json")),
            self.dir.join(format!("{key}.vendor.txt")),
            self.dir.join(format!("{key}.session.json")),
            self.dir.join(format!("{key}.session.txt")),
        )
    }
}

impl TeacherClient for RecordedTeacherClient {
    fn complete(&self, request: &TeacherRequest) -> Result<TeacherCompletion, TeacherError> {
        let (request_path, response_path, vendor_path, session_path, session_binding_path) =
            self.paths(request);
        // Stage then rename: a pre-existing symlink at the request path is
        // replaced by the rename, never followed to truncate its target.
        // Per-process staging name: overlapping reruns never share a tmp file.
        let tmp_path = request_path.with_extension(format!("request.{}.tmp", std::process::id()));
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
        let mut completion = parse_chat_completion(&value)?;
        let response_sha256 = sha256_str(&text);
        apply_vendor_sidecar(&mut completion, &vendor_path, &response_sha256)?;
        if request.requires_session_record {
            apply_session_sidecar(
                &mut completion,
                &session_path,
                &session_binding_path,
                &response_sha256,
            )?;
        }
        Ok(completion)
    }
}

fn apply_session_sidecar(
    completion: &mut TeacherCompletion,
    session_path: &std::path::Path,
    binding_path: &std::path::Path,
    response_sha256: &str,
) -> Result<(), TeacherError> {
    let Some(session) = read_optional_text(session_path, "Hermes session file")? else {
        return Ok(());
    };
    if !valid_session_record(&session) {
        return Ok(());
    }
    let Some(binding) = read_optional_text(binding_path, "Hermes session binding file")? else {
        return Ok(());
    };
    let session_sha256 = sha256_str(&session);
    if !session_binding_matches(&binding, response_sha256, &session_sha256) {
        return Ok(());
    }
    completion.session_file = session_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    completion.session_sha256 = Some(session_sha256);
    Ok(())
}

fn read_optional_text(
    path: &std::path::Path,
    description: &str,
) -> Result<Option<String>, TeacherError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(TeacherError::Transport(format!(
            "cannot read {description} {}: {e}",
            path.display()
        ))),
    }
}

fn session_binding_matches(binding: &str, response_sha256: &str, session_sha256: &str) -> bool {
    let mut parts = binding.split_whitespace();
    let (Some(bound_response), Some(bound_session), None) =
        (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if !bound_response.eq_ignore_ascii_case(response_sha256) {
        return false;
    }
    bound_session.eq_ignore_ascii_case(session_sha256)
}

fn valid_session_record(session: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(session) else {
        return false;
    };
    for key in ["run_id", "session_id", "task_id", "raw_trace_id"] {
        if value[key].as_str().is_none_or(|id| id.trim().is_empty()) {
            return false;
        }
    }
    for key in ["completed", "partial"] {
        if !value[key].is_boolean() {
            return false;
        }
    }
    let Some(messages) = value["messages"].as_array() else {
        return false;
    };
    if messages.is_empty() {
        return false;
    }
    messages.iter().all(|message| {
        message["role"]
            .as_str()
            .is_some_and(|role| !role.trim().is_empty())
            && message["content"].is_string()
    })
}

fn apply_vendor_sidecar(
    completion: &mut TeacherCompletion,
    vendor_path: &std::path::Path,
    response_sha256: &str,
) -> Result<(), TeacherError> {
    if completion
        .served_vendor
        .as_deref()
        .is_some_and(|vendor| !vendor.trim().is_empty())
    {
        return Ok(());
    }
    let vendor = match std::fs::read_to_string(vendor_path) {
        Ok(vendor) => vendor,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(TeacherError::Transport(format!(
                "cannot read vendor file {}: {e}",
                vendor_path.display()
            )));
        }
    };
    let mut parts = vendor.split_whitespace();
    if let (Some(hash), Some(vendor), None) = (parts.next(), parts.next(), parts.next())
        && hash.eq_ignore_ascii_case(response_sha256)
    {
        completion.served_vendor = Some(vendor.to_owned());
    }
    Ok(())
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
    let routing = value.get("routing");
    let served_vendor = [
        routing
            .and_then(|routing| routing.get("vendor_used"))
            .and_then(Value::as_str),
        routing
            .and_then(|routing| routing.get("vendor"))
            .and_then(Value::as_str),
        value.get("vendor").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|vendor| !vendor.trim().is_empty())
    .map(str::to_owned);
    Ok(TeacherCompletion {
        content: content.to_owned(),
        served_model,
        served_vendor,
        session_file: None,
        session_sha256: None,
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
            batch_seed: 0,
            pin_id: "deepseek-v4.1-flash".to_owned(),
            pin_variant: "DeepSeek-V4.1-Flash-1017".to_owned(),
            include_routing_metadata: false,
            requires_session_record: false,
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
        assert!(body.get("include_routing_metadata").is_none());
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
        other.batch_seed = 1 << 32;
        assert_ne!(
            base,
            fulfillment_key(&other),
            "a seed differing only above bit 31 still gets its own files"
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
    fn merge_requests_ask_for_and_parse_routing_vendor_metadata() {
        let dir = FulfillmentDir::new("routing");
        let client = RecordedTeacherClient::new(dir.path.clone()).unwrap();
        let mut request = request();
        request.include_routing_metadata = true;
        let _ = client.complete(&request);
        let key = fulfillment_key(&request);
        let body: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path.join(format!("{key}.request.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(body["include_routing_metadata"], true);

        std::fs::write(
            dir.path.join(format!("{key}.response.json")),
            json!({
                "model": "deepseek/deepseek-v4.1-flash-1017",
                "routing": {"vendor_used": "deepseek"},
                "choices": [{"message": {"content": "{\"ok\":true}"}}]
            })
            .to_string(),
        )
        .unwrap();
        let completion = client.complete(&request).unwrap();
        assert_eq!(completion.served_vendor.as_deref(), Some("deepseek"));

        let response = json!({
            "model": "deepseek/deepseek-v4.1-flash-1017",
            "routing": {"vendor_used": null, "vendor": "  "},
            "choices": [{"message": {"content": "{\"ok\":true}"}}]
        })
        .to_string();
        std::fs::write(dir.path.join(format!("{key}.response.json")), &response).unwrap();
        std::fs::write(
            dir.path.join(format!("{key}.vendor.txt")),
            format!(
                "{} openrouter
",
                sha256_str(&response)
            ),
        )
        .unwrap();
        let completion = client.complete(&request).unwrap();
        assert_eq!(completion.served_vendor.as_deref(), Some("openrouter"));

        std::fs::write(
            dir.path.join(format!("{key}.response.json")),
            format!(
                "{response}
"
            ),
        )
        .unwrap();
        let completion = client.complete(&request).unwrap();
        assert_eq!(completion.served_vendor, None);
    }

    #[test]
    fn session_records_must_be_valid_and_bound_to_the_response() {
        let dir = FulfillmentDir::new("session-binding");
        let client = RecordedTeacherClient::new(dir.path.clone()).unwrap();
        let mut request = request();
        request.requires_session_record = true;
        let _ = client.complete(&request);
        let key = fulfillment_key(&request);
        let response = json!({
            "choices": [{"message": {"content": "{\"ok\":true}"}}]
        })
        .to_string();
        std::fs::write(dir.path.join(format!("{key}.response.json")), &response).unwrap();

        std::fs::write(dir.path.join(format!("{key}.session.json")), "not json").unwrap();
        assert_eq!(client.complete(&request).unwrap().session_file, None);

        let session = "{\"run_id\":\"run-1\",\"session_id\":\"hermes-session-1\",\"task_id\":\"task-1\",\"raw_trace_id\":\"trace-1\",\"completed\":true,\"partial\":false,\"messages\":[{\"role\":\"assistant\",\"content\":\"{}\"}]}";
        std::fs::write(dir.path.join(format!("{key}.session.json")), session).unwrap();
        assert_eq!(client.complete(&request).unwrap().session_file, None);

        std::fs::write(
            dir.path.join(format!("{key}.session.txt")),
            format!("{} {}", sha256_str(&response), sha256_str(session)),
        )
        .unwrap();
        let completion = client.complete(&request).unwrap();
        assert_eq!(
            completion.session_file.as_deref(),
            Some(format!("{key}.session.json").as_str())
        );
        assert_eq!(
            completion.session_sha256.as_deref(),
            Some(sha256_str(session).as_str())
        );

        std::fs::write(
            dir.path.join(format!("{key}.response.json")),
            format!(
                "{response}
"
            ),
        )
        .unwrap();
        let completion = client.complete(&request).unwrap();
        assert_eq!(completion.session_file, None);
        assert_eq!(completion.session_sha256, None);
    }

    #[test]
    fn blank_vendor_fields_fall_through_to_the_next_vendor_field() {
        let completion = parse_chat_completion(&json!({
            "routing": {"vendor_used": null, "vendor": "openrouter"},
            "choices": [{"message": {"content": "{\"ok\":true}"}}]
        }))
        .unwrap();
        assert_eq!(completion.served_vendor.as_deref(), Some("openrouter"));

        let completion = parse_chat_completion(&json!({
            "routing": {"vendor_used": " ", "vendor": " "},
            "vendor": "deepseek",
            "choices": [{"message": {"content": "{\"ok\":true}"}}]
        }))
        .unwrap();
        assert_eq!(completion.served_vendor.as_deref(), Some("deepseek"));
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
