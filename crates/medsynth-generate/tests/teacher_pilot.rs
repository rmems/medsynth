mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{TempDir, plan_paths, read_json, workspace_root};
use medsynth_generate::Lane;
use medsynth_generate::batch::{ExecuteOptions, LaneFilter, execute, load_plan};
use medsynth_generate::format::Format;
use medsynth_generate::hash::sha256_str;
use medsynth_generate::teacher::{
    PreparedTeacher, RecordedTeacherClient, TeacherClient, TeacherError,
};
use serde_json::json;

fn fixture_path(name: &str) -> std::path::PathBuf {
    workspace_root()
        .join("crates/medsynth-generate/tests/fixtures/teacher")
        .join(name)
}

fn fixture_for_prompt(user: &str) -> &'static str {
    if user.contains("- format: case_vignette") {
        "vignette_response.json"
    } else if user.contains("- format: tutoring_example") {
        "tutoring_response.json"
    } else {
        "mcq_response.json"
    }
}

fn write_session_sidecars(dir: &Path, key: &str, response: &str) {
    let response_hash = sha256_str(response);
    let session = serde_json::to_string_pretty(&json!({
        "run_id": format!("test-run-{key}"),
        "session_id": format!("test-session-{key}"),
        "task_id": "teacher-pilot-test",
        "raw_trace_id": format!("test-trace-{key}"),
        "completed": true,
        "partial": false,
        "messages": [
            {"role": "user", "content": "teacher request"},
            {"role": "assistant", "content": "teacher response"}
        ]
    }))
    .unwrap();
    std::fs::write(dir.join(format!("{key}.session.json")), &session).unwrap();
    std::fs::write(
        dir.join(format!("{key}.session.txt")),
        format!("{response_hash} {}", sha256_str(&session)),
    )
    .unwrap();
}

/// Convert every emitted request into a fixture fulfillment with the same raw
/// response/session file contract the external Hermes harness must satisfy.
fn fulfill_requests(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(key) = name.strip_suffix(".request.json") else {
            continue;
        };
        let request = read_json(&path);
        let user = request["messages"][1]["content"].as_str().unwrap();
        let mut response = read_json(&fixture_path(fixture_for_prompt(user)));
        response["vendor"] = json!("deepseek");
        let response_text = serde_json::to_string_pretty(&response).unwrap();
        std::fs::write(dir.join(format!("{key}.response.json")), &response_text).unwrap();
        write_session_sidecars(dir, key, &response_text);
    }
}

#[test]
fn recorded_teacher_replay_produces_schema_valid_drafts_across_all_formats() {
    let config_path = workspace_root().join("drafts/deepseek-pilot/config.json");
    let plan = load_plan(&plan_paths(&config_path), None).unwrap();
    let fulfillment = TempDir::new("teacher-pilot-fulfillment");
    let fulfillment_path = fulfillment.path().to_path_buf();
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(RecordedTeacherClient::new(
            fulfillment_path.clone(),
        )?))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };

    let first = execute(&plan, &options).unwrap();
    assert!(first.lane_error.is_some());
    let request_count = std::fs::read_dir(fulfillment.path())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".request.json")
        })
        .count();
    assert_eq!(request_count, 6);

    fulfill_requests(fulfillment.path());
    let output = execute(&plan, &options).unwrap();
    assert!(output.lane_error.is_none());
    assert_eq!(output.records.len(), 6);
    assert_eq!(output.manifest.rights_envelopes.len(), 6);
    assert_eq!(output.manifest.dropped_by_self_validation, 0);
    assert_eq!(output.manifest.dropped_by_generation, 0);
    let formats: BTreeSet<_> = output
        .records
        .iter()
        .map(|record| record.record.format().slug())
        .collect();
    assert_eq!(
        formats,
        BTreeSet::from(["case_vignette", "mcq", "tutoring_example"])
    );
    assert!(output.records.iter().all(|record| {
        record.record.verification_status() == medsynth_schema::VerificationStatus::Unverified
    }));
}

#[test]
fn deepseek_pilot_config_is_valid_and_covers_all_formats() {
    let root = workspace_root();
    let plan = load_plan(
        &plan_paths(&root.join("drafts/deepseek-pilot/config.json")),
        None,
    )
    .unwrap();
    let formats: BTreeSet<_> = plan.config.cells.iter().map(|cell| cell.format).collect();
    assert_eq!(
        formats,
        BTreeSet::from([Format::Mcq, Format::CaseVignette, Format::TutoringExample,])
    );
    assert!(
        plan.config
            .cells
            .iter()
            .all(|cell| cell.lane == Lane::Teacher && cell.count == 2)
    );
    assert_eq!(
        plan.config.teacher.as_ref().unwrap().pin_id,
        "deepseek-v4.1-flash"
    );
}
