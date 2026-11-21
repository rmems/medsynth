//! Shared test helpers.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use medsynth_generate::batch::{
    BatchOutput, ExecuteOptions, LaneFilter, PlanPaths, execute, load_plan,
};
use medsynth_generate::teacher::{PreparedTeacher, TeacherClient, TeacherError};
use serde_json::Value;

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

pub fn plan_paths(config: &Path) -> PlanPaths {
    let root = workspace_root();
    PlanPaths {
        config: config.to_path_buf(),
        taxonomy: root.join("docs/taxonomy.md"),
        sources: root.join("docs/sources.json"),
        teachers: root.join("docs/teachers.md"),
        schemas: root.join("schemas"),
        gold: root.join("data/gold"),
    }
}

pub fn sample_config() -> PathBuf {
    workspace_root().join("drafts/sample-batch/config.json")
}

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "medsynth-gen-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn write_config(dir: &TempDir, value: &Value) -> PathBuf {
    let path = dir.path().join("config.json");
    std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    path
}

pub fn run_all(plan: &medsynth_generate::BatchPlan) -> BatchOutput {
    let env = |_: &str| None;
    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Err(TeacherError::Parse(
            "tests must inject a fake teacher client".into(),
        ))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    execute(plan, &options).unwrap()
}

pub fn load_sample_plan() -> medsynth_generate::BatchPlan {
    load_plan(&plan_paths(&sample_config()), None).unwrap()
}

pub fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn read_jsonl(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
