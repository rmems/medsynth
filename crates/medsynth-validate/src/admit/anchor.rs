//! Stages 3–4: source anchors and claim support, then the distractor
//! screen. Anchor resolution and registry currency are deterministic;
//! claim support and the distractor verdict come from the pinned
//! verifier via the recorded-fulfillment exchange.

use std::collections::HashMap;

use serde_json::Value;

use crate::Format;

use super::batch::DraftRecord;
use super::claims::{self, ClaimBlock, DistractorScreen};
use super::eval::{
    Ctx, Findings, Verdict, detail_suffix, identity_mismatch, respond, response_is_drafter,
    same_model,
};
use super::fulfill::RequestKind;

/// Stage 3 — deterministic anchor checks plus the verifier's per-claim
/// support verdicts. Returns the verifier response for stage 4's
/// distractor check (`None` when no verifier ran).
pub(super) fn stage_anchor(
    ctx: &Ctx,
    record: &DraftRecord,
) -> Result<(Verdict, Option<Value>), String> {
    let claim_blocks = claims::claim_blocks(record);
    let (mut findings, drifted) = AnchorCheck::new(ctx, record).run(&claim_blocks)?;
    // Claims anchored to drifted entries are re-verified after
    // re-anchoring — the verifier never judges them against passage
    // content the draft did not cite.
    let verifiable: Vec<ClaimBlock> = claim_blocks
        .iter()
        .filter(|b| !drifted.contains(&b.path))
        .cloned()
        .collect();

    // Contract §4's floor, mirroring the CI anchor-presence check: a
    // record that admits with zero registry anchors fails the corpus
    // gate downstream, so the gate rejects it here.
    if registry_anchor_count(&record.value, ctx.registry) == 0 {
        findings.reject(
            "record carries no anchors into the source registry — \
             admitted records require at least one (contract §4)",
        );
    }

    // Semantic support: every claim block gets a verdict from the pinned
    // verifier — supported, claim-free, unsupported, or unanchored. A
    // record with neither claims nor a distractor screen has nothing
    // semantic to check; deterministic results alone decide.
    let distractor = claims::distractor_screen(record);
    if verifiable.is_empty() && distractor.is_none() {
        return Ok((findings.verdict(), None));
    }
    let input = VerifyInput {
        claims: &verifiable,
        distractor: distractor.as_ref(),
    };
    let response = verify_claims(ctx, record, &input, findings)?;
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
    /// Claim blocks whose anchors drifted since drafting — excluded from
    /// the verify request, since a verdict against the replacement
    /// passage proves nothing about the citation as drafted.
    drifted: std::collections::HashSet<String>,
}

impl<'a, 'b> AnchorCheck<'a, 'b> {
    fn new(ctx: &'a Ctx<'b>, record: &'a DraftRecord) -> Self {
        Self {
            ctx,
            record,
            validators: HashMap::new(),
            findings: Findings::default(),
            drifted: std::collections::HashSet::new(),
        }
    }

    fn run(
        mut self,
        blocks: &[ClaimBlock],
    ) -> Result<(Findings, std::collections::HashSet<String>), String> {
        for block in blocks {
            for (source_id, locator) in &block.anchors {
                self.check(block, source_id, locator)?;
            }
        }
        Ok((self.findings, self.drifted))
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
            // The drifted entry's current convention and content prove
            // nothing about the citation as drafted — skip both the
            // receipt and locator checks so a registry change can never
            // read as a record defect.
            self.drifted.insert(block.path.clone());
            return Ok(());
        }
        // The #8 receipt check runs before locator syntax: a stale
        // receipt means the registry entry drifted since drafting, so
        // its current locator convention may not be the one the draft
        // was written against — the record holds for re-anchoring rather
        // than rejecting against a convention it never claimed to meet.
        let anchor = AnchorRef {
            path: &block.path,
            source_id,
            locator,
            sha: entry.sha256.as_str(),
        };
        if self.receipt_stale(&anchor) {
            self.drifted.insert(block.path.clone());
            return Ok(());
        }
        if !self.locator_valid(source_id, locator, &entry.locator_format)? {
            self.findings.reject(format!(
                "{}: locator `{locator}` violates `{source_id}`'s convention `{}`",
                block.path, entry.locator_format
            ));
        }
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

    /// #8's anchor receipts pin the registry entry bytes at draft time;
    /// a drifted or missing receipt holds the record for re-anchoring.
    /// Returns true when the receipt is stale — the anchor is then done:
    /// the drifted entry's current convention proves nothing about the
    /// locator the draft was written against.
    fn receipt_stale(&mut self, anchor: &AnchorRef) -> bool {
        if !self.ctx.batch.has_anchor_receipts() {
            return false;
        }
        match self.ctx.batch.anchor_receipt(
            &self.record.record_id(),
            anchor.source_id,
            anchor.locator,
        ) {
            None => {
                self.findings.hold(format!(
                    "{}: anchor {} {} has no batch receipt",
                    anchor.path, anchor.source_id, anchor.locator
                ));
                true
            }
            Some(receipt) => {
                let stale = receipt["registry_entry_sha256"].as_str() != Some(anchor.sha);
                if stale {
                    self.findings.hold(format!(
                        "{}: registry entry `{}` changed since drafting — \
                         held for re-anchoring",
                        anchor.path, anchor.source_id
                    ));
                }
                stale
            }
        }
    }
}

/// One anchor's context: the claim path it sits under, the cited source
/// and locator, and the registry entry hash it must match.
struct AnchorRef<'a> {
    path: &'a str,
    source_id: &'a str,
    locator: &'a str,
    sha: &'a str,
}

/// The semantic surface the verifier judges: the claim blocks plus the
/// optional distractor screen.
struct VerifyInput<'a> {
    claims: &'a [ClaimBlock],
    distractor: Option<&'a DistractorScreen>,
}

/// The verifier exchange: send the claim blocks and distractor screen,
/// fold each returned verdict into findings, and hand the response back
/// for stage 4's distractor check.
fn verify_claims(
    ctx: &Ctx,
    record: &DraftRecord,
    input: &VerifyInput,
    mut findings: Findings,
) -> Result<(Verdict, Option<Value>), String> {
    // The verifier must never be the drafting model — checked before any
    // request is emitted, the same rule the stage-2 solver follows.
    let verifier = ctx.config.verifier.as_ref().expect("check_pins ran");
    if verifier
        .model_id
        .as_deref()
        .is_some_and(|m| same_model(ctx, record, m, None))
    {
        findings.hold("stage-3/4 verifier is the model that drafted this record");
        return Ok((findings.verdict(), None));
    }
    let mut body = claims::verify_body(&ctx.scope(), record, input.claims, input.distractor);
    body["source_pins"] = source_pins(ctx.registry, input.claims);
    let response = match respond(ctx, RequestKind::Verify, &body, "verifier")? {
        Ok(value) => value,
        Err(reason) => {
            findings.hold(reason);
            return Ok((findings.verdict(), None));
        }
    };
    // A response from the wrong run/model is untrusted: hold for a
    // correct fulfillment and stop — its claim verdicts must not reject
    // or support anything.
    if let Some(reason) = identity_mismatch(
        &response,
        &verifier.run_id,
        verifier.model_id.as_deref(),
        "verifier",
    ) {
        findings.hold(reason);
        return Ok((findings.verdict(), None));
    }
    // The verifier must be independent of the drafting run — the response
    // names its model even when the pin does not.
    if let Some(reason) = response_is_drafter(ctx, record, &response, "verifier") {
        findings.hold(reason);
        return Ok((findings.verdict(), None));
    }
    // Every returned verdict applies — a duplicated path keeps all its
    // verdicts, so a `supported` entry can never conceal an `unsupported`
    // one for the same claim.
    let mut verdicts: HashMap<&str, Vec<&Value>> = HashMap::new();
    for v in response["claims"].as_array().into_iter().flatten() {
        if let Some(path) = v["path"].as_str() {
            verdicts.entry(path).or_default().push(v);
        }
    }
    for block in input.claims {
        let vs = verdicts
            .get(block.path.as_str())
            .map_or(&[][..], Vec::as_slice);
        apply_verdicts(block, vs, &mut findings);
    }
    Ok((findings.verdict(), Some(response)))
}

/// Fold every verdict the verifier returned for one claim block — none
/// holds, any number fold in order so a rejection can never hide.
fn apply_verdicts(block: &ClaimBlock, vs: &[&Value], findings: &mut Findings) {
    if vs.is_empty() {
        findings.hold(format!("verifier returned no verdict for {}", block.path));
    }
    for v in vs {
        apply_claim_verdict(block, v, findings);
    }
}

fn apply_claim_verdict(block: &ClaimBlock, v: &Value, findings: &mut Findings) {
    // `supported` requires a citation to be supported by — on an
    // anchor-less block it is an unusable response, not a pass.
    if v["verdict"].as_str() == Some("supported") && block.anchors.is_empty() {
        findings.hold(format!(
            "{}: verifier claims `supported` but the block cites no source",
            block.path
        ));
        return;
    }
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

/// `{source_id: entry sha256}` for every cited registry entry — binds the
/// verify request to the bytes actually checked, so a changed entry
/// produces a different fulfillment key and a stale `supported` can
/// never answer a current request.
fn source_pins(registry: &super::registry::SourceRegistry, claims_list: &[ClaimBlock]) -> Value {
    claims_list
        .iter()
        .flat_map(|c| c.anchors.iter().map(|a| &a.0))
        .filter_map(|id| {
            registry
                .get(id)
                .map(|e| (id.clone(), e.sha256.clone().into()))
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

/// Count `anchors[]` entries anywhere in the record citing a registered
/// source id — the same walk the CI `anchor-presence` check performs.
fn registry_anchor_count(value: &Value, registry: &super::registry::SourceRegistry) -> usize {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                if k == "anchors" {
                    v.as_array()
                        .map(|anchors| {
                            anchors
                                .iter()
                                .filter(|a| {
                                    a["source_id"]
                                        .as_str()
                                        .is_some_and(|id| registry.get(id).is_some())
                                })
                                .count()
                        })
                        .unwrap_or(0)
                } else {
                    registry_anchor_count(v, registry)
                }
            })
            .sum(),
        Value::Array(items) => items
            .iter()
            .map(|v| registry_anchor_count(v, registry))
            .sum(),
        _ => 0,
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
