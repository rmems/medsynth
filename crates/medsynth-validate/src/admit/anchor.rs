//! Stages 3–4: source anchors and claim support, then the distractor
//! screen. Anchor resolution and registry currency are deterministic;
//! claim support and the distractor verdict come from the pinned
//! verifier via the recorded-fulfillment exchange.

use std::collections::HashMap;

use serde_json::Value;

use crate::Format;

use super::batch::DraftRecord;
use super::claims::{self, ClaimBlock, DistractorScreen};
use super::eval::{Ctx, Findings, Verdict, detail_suffix, identity_mismatch, respond};
use super::fulfill::RequestKind;

/// Stage 3 — deterministic anchor checks plus the verifier's per-claim
/// support verdicts. Returns the verifier response for stage 4's
/// distractor check (`None` when no verifier ran).
pub(super) fn stage_anchor(
    ctx: &Ctx,
    record: &DraftRecord,
) -> Result<(Verdict, Option<Value>), String> {
    let claim_blocks = claims::claim_blocks(record);
    let findings = AnchorCheck::new(ctx, record).run(&claim_blocks)?;

    // Semantic support: every claim block gets a verdict from the pinned
    // verifier — supported, claim-free, unsupported, or unanchored. A
    // record with neither claims nor a distractor screen has nothing
    // semantic to check; deterministic results alone decide.
    let distractor = claims::distractor_screen(record);
    if claim_blocks.is_empty() && distractor.is_none() {
        return Ok((findings.verdict(), None));
    }
    let response = verify_claims(ctx, record, &claim_blocks, distractor.as_ref(), findings)?;
    Ok((response.0, response.1))
}

/// One record's deterministic anchor pass: registry membership and
/// currency, locator syntax under the entry's convention, and the #8
/// anchor receipts. Locator validators are compiled once per source id.
struct AnchorCheck<'a, 'b> {
    ctx: &'a Ctx<'b>,
    record: &'a DraftRecord,
    validators: HashMap<String, jsonschema::Validator>,
    findings: Findings,
}

impl<'a, 'b> AnchorCheck<'a, 'b> {
    fn new(ctx: &'a Ctx<'b>, record: &'a DraftRecord) -> Self {
        Self {
            ctx,
            record,
            validators: HashMap::new(),
            findings: Findings::default(),
        }
    }

    fn run(mut self, blocks: &[ClaimBlock]) -> Result<Findings, String> {
        for block in blocks {
            for (source_id, locator) in &block.anchors {
                self.check(block, source_id, locator)?;
            }
        }
        Ok(self.findings)
    }

    fn check(&mut self, block: &ClaimBlock, source_id: &str, locator: &str) -> Result<(), String> {
        let Some(entry) = self.ctx.registry.get(source_id) else {
            self.findings.reject(format!(
                "{}: cites `{source_id}`, which is not in the source registry",
                block.path
            ));
            return Ok(());
        };
        if !entry.is_current() {
            self.findings.hold(format!(
                "{}: `{source_id}` is `{}` — held for re-anchoring",
                block.path, entry.status
            ));
        }
        if !self.locator_valid(source_id, locator, &entry.locator_format)? {
            self.findings.reject(format!(
                "{}: locator `{locator}` violates `{source_id}`'s convention `{}`",
                block.path, entry.locator_format
            ));
        }
        self.check_receipt(block, source_id, locator, entry.sha256.as_str());
        Ok(())
    }

    /// The locator's syntax under the entry's convention — validators are
    /// cached per source id.
    fn locator_valid(
        &mut self,
        source_id: &str,
        locator: &str,
        locator_format: &str,
    ) -> Result<bool, String> {
        let validator = match self.validators.entry(source_id.to_owned()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let pattern = medsynth_schema::locator::pattern(locator_format).ok_or_else(|| {
                    format!("registry entry `{source_id}` has an unrecognized locator convention `{locator_format}`")
                })?;
                e.insert(
                    jsonschema::validator_for(&serde_json::json!({
                        "type": "string",
                        "pattern": pattern,
                    }))
                    .expect("locator patterns compile"),
                )
            }
        };
        Ok(validator.is_valid(&serde_json::json!(locator)))
    }

    /// #8's anchor receipts pin the registry entry bytes at draft time; a
    /// drifted entry holds the record for re-anchoring rather than
    /// rejecting it.
    fn check_receipt(&mut self, block: &ClaimBlock, source_id: &str, locator: &str, sha: &str) {
        if !self.ctx.batch.has_anchor_receipts() {
            return;
        }
        match self
            .ctx
            .batch
            .anchor_receipt(&self.record.record_id(), source_id, locator)
        {
            None => self.findings.hold(format!(
                "{path}: anchor {source_id} {locator} has no batch receipt",
                path = block.path
            )),
            Some(receipt) if receipt["registry_entry_sha256"].as_str() != Some(sha) => {
                self.findings.hold(format!(
                    "{}: registry entry `{source_id}` changed since drafting — \
                     held for re-anchoring",
                    block.path
                ));
            }
            Some(_) => {}
        }
    }
}

/// The verifier exchange: send the claim blocks and distractor screen,
/// fold each returned verdict into findings, and hand the response back
/// for stage 4's distractor check.
fn verify_claims(
    ctx: &Ctx,
    record: &DraftRecord,
    claim_blocks: &[ClaimBlock],
    distractor: Option<&DistractorScreen>,
    mut findings: Findings,
) -> Result<(Verdict, Option<Value>), String> {
    let body = claims::verify_body(&ctx.batch.batch_id, record, claim_blocks, distractor);
    let response = match respond(ctx, RequestKind::Verify, &body, "verifier")? {
        Ok(value) => value,
        Err(reason) => {
            findings.hold(reason);
            return Ok((findings.verdict(), None));
        }
    };
    let verifier_run_id = &ctx.config.verifier.as_ref().expect("check_pins ran").run_id;
    if let Some(reason) = identity_mismatch(&response, verifier_run_id, None, "verifier") {
        findings.hold(reason);
    }
    let verdicts: HashMap<&str, &Value> = response["claims"]
        .as_array()
        .map(|vs| {
            vs.iter()
                .filter_map(|v| v["path"].as_str().map(|p| (p, v)))
                .collect()
        })
        .unwrap_or_default();
    for block in claim_blocks {
        match verdicts.get(block.path.as_str()) {
            None => findings.hold(format!("verifier returned no verdict for {}", block.path)),
            Some(v) => apply_claim_verdict(block, v, &mut findings),
        }
    }
    Ok((findings.verdict(), Some(response)))
}

fn apply_claim_verdict(block: &ClaimBlock, v: &Value, findings: &mut Findings) {
    match v["verdict"].as_str() {
        Some("supported") | Some("no-claim") => {}
        Some("unanchored") => findings.reject(format!(
            "{}: unanchored factual claim{}",
            block.path,
            detail_suffix(v)
        )),
        Some("unsupported") => findings.reject(format!(
            "{}: claim not supported by the cited passage{}",
            block.path,
            detail_suffix(v)
        )),
        other => findings.hold(format!(
            "verifier returned unknown verdict `{}` for {}",
            other.unwrap_or("<missing>"),
            block.path
        )),
    }
}

/// Stage 4 — the distractor plausibility screen, read off the stage-3
/// verifier response.
pub(super) fn stage_distractor(record: &DraftRecord, verify_response: Option<&Value>) -> Verdict {
    if record.format != Format::Mcq {
        return Verdict::Pass;
    }
    // The statable-reason requirement: a blank refutation never survives
    // stage 1, but the gate re-checks rather than trusting the schema.
    if record.value["rationale"]["strongest_distractor"]["text"]
        .as_str()
        .is_none_or(|t| t.trim().is_empty())
    {
        return Verdict::reject("rationale records no reason the strongest distractor is wrong");
    }
    let Some(response) = verify_response else {
        return Verdict::hold("no verifier response for the distractor screen");
    };
    match response["distractor"]["verdict"].as_str() {
        Some("pass") => Verdict::Pass,
        Some("fail") => Verdict::reject(format!(
            "distractor screen{}",
            detail_suffix(&response["distractor"])
        )),
        _ => Verdict::hold("verifier returned no distractor verdict"),
    }
}
