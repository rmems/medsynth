mod common;

use std::process::Command;

use common::{TempDir, workspace_root, write_config};
use serde_json::{Value, json};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_medsynth-generate"));
    command.current_dir(workspace_root());
    command
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn templates_matrix_and_check_pin_commands_work() {
    let output = command().arg("templates").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("fact.alveolar_epithelium"));
    assert!(text.contains("calc.half_life"));

    let output = command().arg("matrix").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("taxonomy version: 0.1"));
    assert!(text.contains("total target: 300"));

    let output = command()
        .args(["check-pin", "--pin", "deepseek-v4.1-flash"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("teacher-ready"));

    let output = command()
        .args(["check-pin", "--pin", "not-a-pin"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("no pin"));
}

#[test]
fn run_command_writes_a_batch_and_refuses_data_directory() {
    let config = json!({
        "seed": 9u64,
        "cells": [{
            "domain": "cpf",
            "format": "mcq",
            "tier": "application",
            "count": 1,
            "lane": "procedural",
            "template": "calc.newtons_second_law",
            "subdomain": "mechanics"
        }]
    });
    let config_dir = TempDir::new("cli-config");
    let config_path = write_config(&config_dir, &config);
    let out = TempDir::new("cli-out");
    let output = command()
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(out.path().join("mcq.jsonl").exists());
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["seed"], 9);
    assert_eq!(manifest["records"].as_array().unwrap().len(), 1);

    let bad_out = workspace_root().join("data/mcq");
    let output = command()
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--out",
            bad_out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("refusing to write drafts under"));
}

#[test]
fn run_command_reports_teacher_refusal_and_keeps_procedural_output() {
    let config = json!({
        "seed": 10u64,
        "cells": [
            {
                "domain": "cpf",
                "format": "mcq",
                "tier": "application",
                "count": 1,
                "lane": "procedural",
                "template": "calc.newtons_second_law",
                "subdomain": "mechanics"
            },
            {
                "domain": "bbf",
                "format": "case_vignette",
                "tier": "foundation",
                "count": 1,
                "lane": "teacher",
                "subdomain": "animal_structure"
            }
        ],
        "teacher": {"pin_id": "deepseek-v4.1-flash"}
    });
    let config_dir = TempDir::new("cli-teacher-config");
    let config_path = write_config(&config_dir, &config);
    let out = TempDir::new("cli-teacher-out");
    // No --teacher-responses: the teacher lane must refuse — the pipeline
    // never calls teacher APIs itself.
    let output = command()
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = stderr(&output);
    assert!(error.contains("teacher lane did not run"), "{error}");
    assert!(error.contains("fulfillment directory"), "{error}");
    assert!(out.path().join("mcq.jsonl").exists());
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["teacher_lane"]["status"], "refused");
    assert_eq!(manifest["totals"]["produced"], 1);

    // The fulfillment directory is held to the same `data/` restriction.
    let out2 = TempDir::new("cli-teacher-out2");
    let output = command()
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--out",
            out2.path().to_str().unwrap(),
            "--teacher-responses",
            workspace_root().join("data/fulfill").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("refusing to write drafts under"),
        "{}",
        stderr(&output)
    );

    // A `--lane procedural` run never touches the fulfillment directory, so
    // the same `data/` path must not block it.
    let out3 = TempDir::new("cli-procedural-out");
    let output = command()
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--out",
            out3.path().to_str().unwrap(),
            "--lane",
            "procedural",
            "--teacher-responses",
            workspace_root().join("data/fulfill").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(out3.path().join("mcq.jsonl").exists());
}
