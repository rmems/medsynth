mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{
    TempDir, load_sample_plan, plan_paths, read_jsonl, run_all, sample_config, workspace_root,
    write_config,
};
use medsynth_generate::config::Lane;
use medsynth_generate::hash::sha256_str;
use medsynth_generate::record::record_id_for;
use medsynth_generate::rng::SeededRng;
use medsynth_generate::{Format, GeneratedRecord, TargetCell, load_plan, procedural};
use medsynth_schema::{Domain, McqRecord, Subdomain, Tier, VerificationStatus};
use serde_json::json;

fn cell(domain: Domain, tier: Tier, subdomain: Subdomain, template: &str) -> TargetCell {
    TargetCell {
        domain,
        format: Format::Mcq,
        tier,
        count: 1,
        lane: Lane::Procedural,
        template: Some(template.to_owned()),
        subdomain,
        resolved_attempts: 0,
        in_flight_attempts: 0,
    }
}

fn build_calc(template_id: &str, cell: &TargetCell, seed: u64) -> McqRecord {
    let id = record_id_for(seed, cell, 0).unwrap();
    let mut rng = SeededRng::new(seed, &["test", template_id, &seed.to_string()]);
    match procedural::build(template_id, cell, id, &mut rng).unwrap() {
        GeneratedRecord::Mcq(record) => record,
        other => panic!("expected mcq, got {}", other.format()),
    }
}

fn keyed_text(record: &McqRecord) -> String {
    record
        .options
        .iter()
        .find(|option| option.label == record.key)
        .unwrap()
        .text
        .clone()
}

fn number_after(text: &str, prefix: &str, suffix: &str) -> u64 {
    let start = text
        .find(prefix)
        .unwrap_or_else(|| panic!("missing prefix `{prefix}` in `{text}`"))
        + prefix.len();
    let rest = &text[start..];
    let end = rest
        .find(suffix)
        .unwrap_or_else(|| panic!("missing suffix `{suffix}` after `{prefix}` in `{text}`"));
    rest[..end].trim().parse().unwrap()
}

#[test]
fn same_seed_and_config_reproduce_bytes() {
    let plan = load_plan(&plan_paths(&sample_config()), None).unwrap();
    let first = run_all(&plan);
    let second = run_all(&plan);

    assert_eq!(first.records.len(), 8);
    assert_eq!(second.records.len(), first.records.len());
    for (a, b) in first.records.iter().zip(second.records.iter()) {
        assert_eq!(a.record.id(), b.record.id());
        assert_eq!(a.raw, b.raw, "record `{}` differs", a.record.id());
    }
    assert_eq!(
        first.manifest.to_json_string().unwrap(),
        second.manifest.to_json_string().unwrap()
    );
}

#[test]
fn calculation_keys_recompute_independently() {
    let newton = cell(
        Domain::Cpf,
        Tier::Application,
        Subdomain::Mechanics,
        "calc.newtons_second_law",
    );
    for seed in 0..60u64 {
        let record = build_calc("calc.newtons_second_law", &newton, seed);
        let stem = &record.stem.text;
        let force = number_after(stem, "net force of ", " N");
        let mass = number_after(stem, "mass ", " kg");
        let expected = force / mass;
        assert_eq!(
            keyed_text(&record),
            format!("{expected} m/s^2"),
            "seed {seed}: {stem}"
        );
        assert_eq!(
            record
                .options
                .iter()
                .filter(|o| o.text == keyed_text(&record))
                .count(),
            1
        );
    }

    let half_life = cell(
        Domain::PreClinical,
        Tier::Application,
        Subdomain::PharmacologyPrinciples,
        "calc.half_life",
    );
    for seed in 0..60u64 {
        let record = build_calc("calc.half_life", &half_life, seed);
        let stem = &record.stem.text;
        let initial = number_after(stem, "single ", " mg");
        let half_life_hours = number_after(stem, "half-life is ", " hours");
        let elapsed = number_after(stem, "remains ", " hours");
        let half_lives = elapsed / half_life_hours;
        let expected = initial >> u32::try_from(half_lives).unwrap();
        assert_eq!(
            keyed_text(&record),
            format!("{expected} mg"),
            "seed {seed}: {stem}"
        );
    }
}

#[test]
fn fact_keys_come_from_the_anchored_fact() {
    for fact in procedural::FACTS {
        let cell = TargetCell {
            domain: fact.domain,
            format: Format::Mcq,
            tier: fact.tier,
            count: 1,
            lane: Lane::Procedural,
            template: Some(fact.id.to_owned()),
            subdomain: fact.subdomain,
            resolved_attempts: 0,
            in_flight_attempts: 0,
        };
        for seed in 0..30u64 {
            let record = build_calc(fact.id, &cell, seed);
            assert_eq!(
                keyed_text(&record),
                fact.correct,
                "template {} seed {seed}",
                fact.id
            );
            assert!(
                record.options.iter().any(|o| o.label == record.key),
                "key must match an option"
            );
            assert_eq!(record.options.len(), 4);
            assert_eq!(
                record
                    .options
                    .iter()
                    .filter(|option| option.text == fact.correct)
                    .count(),
                1,
                "template {} must have one option for the fact's answer",
                fact.id
            );
            let texts: BTreeSet<&str> = record
                .options
                .iter()
                .map(|option| option.text.as_str())
                .collect();
            assert_eq!(
                texts.len(),
                record.options.len(),
                "option text must be unique"
            );
        }
    }
}

#[test]
fn provenance_is_complete_and_receipts_cover_every_anchor() {
    let output = run_all(&load_sample_plan());
    let manifest = &output.manifest;
    assert_eq!(manifest.dropped_by_self_validation, 0);
    assert_eq!(manifest.dropped_by_generation, 0);
    assert_eq!(manifest.records.len(), output.records.len());

    let mut domains: BTreeMap<String, usize> = BTreeMap::new();
    for record in &output.records {
        let provenance = record.record.provenance();
        assert!(
            provenance.generator.starts_with("template:"),
            "{}",
            provenance.generator
        );
        assert!(
            provenance
                .template_version
                .as_deref()
                .is_some_and(|version| !version.trim().is_empty()),
            "record {} lacks template_version",
            record.record.id()
        );
        assert!(provenance.model.is_none());
        assert!(provenance.prompt_version.is_none());
        assert_eq!(
            record.record.verification_status(),
            VerificationStatus::Unverified
        );
        assert!(!record.record.anchors().is_empty());

        let id = record.record.id().as_str();
        let entry = manifest
            .records
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("manifest lacks record `{id}`"));
        assert_eq!(entry.sha256, sha256_str(&record.raw));
        for (source_id, locator) in record.record.anchors() {
            assert!(
                manifest.anchor_receipts.iter().any(|receipt| {
                    receipt.record_id == id
                        && receipt.source_id == source_id
                        && receipt.locator == locator
                }),
                "missing receipt for {id} {source_id} {locator}"
            );
        }
        *domains
            .entry(medsynth_generate::matrix::domain_slug(record.record.domain()).to_owned())
            .or_default() += 1;
    }
    assert_eq!(domains.len(), 4);
    for count in domains.values() {
        assert!(*count >= 2);
    }
}

#[test]
fn budget_cap_limits_dispatch_and_escalates() {
    let capped = json!({
        "seed": 7u64,
        "cells": [{
            "domain": "bbf",
            "format": "mcq",
            "tier": "foundation",
            "count": 18,
            "lane": "procedural",
            "template": "fact.alveolar_epithelium",
            "subdomain": "animal_structure",
            "resolved_attempts": 30,
            "in_flight_attempts": 10
        }]
    });
    let dir = TempDir::new("budget-capped");
    let config = write_config(&dir, &capped);
    let plan = load_plan(&plan_paths(&config), None).unwrap();
    let output = run_all(&plan);
    let cell = &output.manifest.cells[0];
    assert_eq!(cell.matrix_target, 18);
    assert_eq!(cell.budget_cap, 54);
    assert_eq!(cell.resolved_attempts, 30);
    assert_eq!(cell.in_flight_attempts, 10);
    assert_eq!(cell.dispatched, 14);
    assert_eq!(cell.produced, 14);
    assert!(cell.budget_limited);
    assert!(!cell.escalated);
    assert_eq!(output.manifest.dropped_by_self_validation, 0);

    let replacement = json!({
        "seed": 7u64,
        "cells": [{
            "domain": "bbf",
            "format": "mcq",
            "tier": "foundation",
            "count": 54,
            "lane": "procedural",
            "template": "fact.alveolar_epithelium",
            "subdomain": "animal_structure",
            "resolved_attempts": 40,
            "in_flight_attempts": 0
        }]
    });
    let dir = TempDir::new("budget-replacement");
    let config = write_config(&dir, &replacement);
    let plan = load_plan(&plan_paths(&config), None).unwrap();
    let output = run_all(&plan);
    let cell = &output.manifest.cells[0];
    assert_eq!(cell.requested, 54);
    assert_eq!(cell.dispatched, 14);
    assert_eq!(cell.produced, 14);
    assert!(cell.budget_limited);
    assert!(!cell.escalated);

    let exhausted = json!({
        "seed": 7u64,
        "cells": [{
            "domain": "bbf",
            "format": "mcq",
            "tier": "foundation",
            "count": 18,
            "lane": "procedural",
            "template": "fact.alveolar_epithelium",
            "subdomain": "animal_structure",
            "resolved_attempts": 54,
            "in_flight_attempts": 0
        }]
    });
    let dir = TempDir::new("budget-exhausted");
    let config = write_config(&dir, &exhausted);
    let plan = load_plan(&plan_paths(&config), None).unwrap();
    let output = run_all(&plan);
    let cell = &output.manifest.cells[0];
    assert_eq!(cell.dispatched, 0);
    assert_eq!(cell.produced, 0);
    assert!(cell.budget_limited);
    assert!(cell.escalated);
    assert_eq!(cell.status, "escalated");
    assert!(output.records.is_empty());
}

#[test]
fn output_directory_may_not_be_under_data() {
    let output = run_all(&load_sample_plan());
    let root = workspace_root();
    let data_error = output.write_output(&root.join("data/mcq")).unwrap_err();
    assert!(
        data_error
            .to_string()
            .contains("refusing to write drafts under")
    );
    let gold_error = output.write_output(&root.join("data/gold")).unwrap_err();
    assert!(
        gold_error
            .to_string()
            .contains("refusing to write drafts under")
    );

    let dir = TempDir::new("write-output");
    let written = output.write_output(dir.path()).unwrap();
    assert!(written.iter().any(|path| path.ends_with("mcq.jsonl")));
    assert!(written.iter().any(|path| path.ends_with("manifest.json")));
    assert_eq!(read_jsonl(&dir.path().join("mcq.jsonl")).len(), 8);
}
