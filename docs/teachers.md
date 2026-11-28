# Teacher pin registry

**Status:** binding for the teacher lane of the generation pipeline (#8).
**Snapshot taken:** 2026-11-27, before any pilot teacher output was generated.
**Machine-read by:** `crates/medsynth-generate` (`teacher::pin`). The teacher
lane refuses to start unless the selected entry below is present, complete,
classified as a teacher, drawn from an approved family, and backed by terms
that permit distillation and synthetic-data generation.

This file is the only pin source. A teacher not listed here is unpinned, and an
unpinned teacher cannot draft. Adding or changing an entry is a reviewed change
that must be snapshotted before the next teacher run. The parser reads the
`### Pin:` blocks and the `- **key:** \`value\`` fields; prose outside those
blocks is documentation, not gate input.

## Snapshot record

Terms pages and model cards were read on **2026-11-27**. Terms can change;
re-snapshot and re-review before a later run. The exact clause relied on is
quoted in each entry, so the basis is auditable without re-fetching the page.

| Teacher / serving route | Terms document | Version / date | URL |
| --- | --- | --- | --- |
| DeepSeek V4.1 Flash | DeepSeek Open Platform Terms of Service | Release 2026-04-22; effective 2026-04-29 | <https://cdn.deepseek.com/policies/en-US/deepseek-open-platform-terms-of-service.html> |
| Merge Gateway serving route | Merge Gateway Terms | Last updated 2026-04-02 | <https://www.merge.dev/legal/gateway-terms> |
| OpenRouter serving route | OpenRouter Terms of Service | Last updated 2026-08-31 | <https://openrouter.ai/terms> |
| NVIDIA Nemotron 3 Ultra | NVIDIA Nemotron Open Model License | v. December 15, 2025 | <https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-nemotron-open-model-license/> |

Merge Gateway's routing documentation is part of the route receipt rather than
a license basis: requests may set `include_routing_metadata`, and the response's
`routing.vendor_used` field (or the `x-merge-vendor` header in a harness that
preserves headers) identifies the serving vendor.

Hermes is the execution route for teacher generation: DeepSeek V4.1 Flash
drives the Hermes sessions, and each `<key>.session.json` record is part of
the fulfillment set. Rights envelopes therefore name the full path — teacher,
Hermes, serving route, and serving vendor — and fail closed when the session
record is absent.

## Teacher pins

### Pin: `deepseek-v4.1-flash`

- **pin_id:** `deepseek-v4.1-flash`
- **provider:** `DeepSeek`
- **route:** `Hermes via Merge Gateway`
- **family:** `deepseek`
- **variant:** `DeepSeek-V4.1-Flash-1017`
- **model_id:** `deepseek/deepseek-v4.1-flash-1017`
- **endpoint:** `https://api-gateway.merge.dev/v1/chat/completions`
- **api_key_env:** `MERGE_GATEWAY_API_KEY`
- **lane:** `teacher`
- **terms_document:** `DeepSeek Open Platform Terms of Service`
- **terms_version:** `release 2026-04-22; effective 2026-04-29`
- **terms_date:** `2026-04-29`
- **terms_url:** `https://cdn.deepseek.com/policies/en-US/deepseek-open-platform-terms-of-service.html`
- **terms_clause:** `§4.2(3): "You may apply the Inputs and Outputs of the Services to a wide range of use cases, including personal use, academic research, derivative product development, training other models (such as model distillation), etc."`
- **route_terms_document:** `Merge Gateway Terms`
- **route_terms_version:** `last updated 2026-04-02`
- **route_terms_date:** `2026-04-02`
- **route_terms_url:** `https://www.merge.dev/legal/gateway-terms`
- **route_terms_clause:** `§1: Customer Data includes Inputs and "any outputs received through the Service from AI Providers ... based on such Inputs". §5.1: Customer grants Merge only a limited license to use Customer Data to provide and maintain the Service and develop Usage Data; except when payload logging is enabled, Merge does not retain Inputs or Outputs. §4.2: Customer must comply with the terms for each AI Provider used through the Service.`
- **permits_distillation:** `yes`
- **permits_synthetic_data:** `yes`
- **snapshot_date:** `2026-11-27`
- **notes:** `Variant as updated 2026-10-17 (DeepSeek-V4.1-Flash-1017), addressed through the exact Merge catalog id deepseek/deepseek-v4.1-flash-1017. Outputs are assigned to the developer under DeepSeek §4.2(2) and may be used for model distillation under §4.2(3). The serving vendor must be captured from Merge routing metadata in each rights envelope.`

### Pin: `deepseek-v4.1-flash-openrouter`

- **pin_id:** `deepseek-v4.1-flash-openrouter`
- **provider:** `DeepSeek`
- **route:** `Hermes via OpenRouter`
- **family:** `deepseek`
- **variant:** `DeepSeek-V4.1-Flash-1017`
- **model_id:** `deepseek/deepseek-v4.1-flash-1017`
- **endpoint:** `https://openrouter.ai/api/v1/chat/completions`
- **api_key_env:** `OPENROUTER_API_KEY`
- **lane:** `teacher`
- **terms_document:** `DeepSeek Open Platform Terms of Service`
- **terms_version:** `release 2026-04-22; effective 2026-04-29`
- **terms_date:** `2026-04-29`
- **terms_url:** `https://cdn.deepseek.com/policies/en-US/deepseek-open-platform-terms-of-service.html`
- **terms_clause:** `§4.2(3): "You may apply the Inputs and Outputs of the Services to a wide range of use cases, including personal use, academic research, derivative product development, training other models (such as model distillation), etc."`
- **route_terms_document:** `OpenRouter Terms of Service`
- **route_terms_version:** `last updated 2026-08-31`
- **route_terms_date:** `2026-08-31`
- **route_terms_url:** `https://openrouter.ai/terms`
- **route_terms_clause:** `§5.1: users are responsible for determining whether applicable Model Terms allow use of Inputs and Outputs as intended. §5.8: OpenRouter does not modify, waive, or limit any Model Terms unless expressly stated in writing. §6.1: ownership rights in the Output are set forth in the Model Terms for each Model used.`
- **permits_distillation:** `yes`
- **permits_synthetic_data:** `yes`
- **snapshot_date:** `2026-11-27`
- **notes:** `Alternate serving route for the same fixed 2026-10-17 variant and exact catalog id as deepseek-v4.1-flash. OpenRouter's catalog snapshot on 2026-11-27 listed deepseek/deepseek-v4.1-flash-1017 as the fixed 1017 snapshot. The generated request uses the exact catalog id and restricts OpenRouter to the "deepseek" provider with fallbacks disabled. The harness must enable OpenRouter routing metadata and preserve the selected serving provider in each rights envelope; the floating latest alias and 0731 variant remain unpinned.`

### Pin: `nemotron-3-ultra`

- **pin_id:** `nemotron-3-ultra`
- **provider:** `NVIDIA NIM (build.nvidia.com)`
- **route:** `NVIDIA NIM`
- **family:** `nvidia-nemotron`
- **variant:** `NVIDIA-Nemotron-3-Ultra-550B-A55B-NVFP4`
- **model_id:** `nvidia/nemotron-3-ultra-550b-a55b`
- **endpoint:** `https://integrate.api.nvidia.com/v1/chat/completions`
- **api_key_env:** `NVIDIA_API_KEY`
- **lane:** `teacher`
- **terms_document:** `NVIDIA Nemotron Open Model License`
- **terms_version:** `v. December 15, 2025`
- **terms_date:** `2025-12-15`
- **terms_url:** `https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-nemotron-open-model-license/`
- **terms_clause:** `Preamble: "NVIDIA does not claim ownership to any outputs generated using the Works or Derivative Works." §2 Grant of License: "perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable license to reproduce, prepare Derivative Works of, publicly display, publicly perform, sublicense, and distribute the Work and such Derivative Works in source or object form."`
- **permits_distillation:** `yes`
- **permits_synthetic_data:** `yes`
- **snapshot_date:** `2026-11-27`
- **notes:** `Nemotron 3 family, Ultra 550B A55B NVFP4 variant served directly by NVIDIA NIM. The license does not claim ownership of outputs and grants derivative-work rights, so outputs may be used to build synthetic training data. Price-gated for the pilot: run only through a free route; otherwise pinned but unrun.`

## Never teachers

These are recorded here so the exclusion is visible, not because they are
pins. Their outputs are research-only for this repository and are never used
as teacher drafts:

- **Grok (xAI)** — research-only. Not an approved teacher family.
- **Claude (Anthropic)** — research-only. Not an approved teacher family.
- **Xiaomi MiMo V2.6 Flash** — removed from the teacher roster on 2026-11-27. Its MIT weights license does not grant rights in hosted API output, so it is not an approved teacher.

A pin entry whose `lane` is not `teacher`, or whose `family` is neither
`deepseek` nor `nvidia-nemotron`, is rejected by the gate even if its terms
look permissive. The gate also rejects an entry that omits any required field
or that does not affirm both `permits_distillation` and
`permits_synthetic_data`, and an entry whose `endpoint` is not an approved
provider host (`api.deepseek.com`, `integrate.api.nvidia.com`,
`api-gateway.merge.dev`, `openrouter.ai`) or whose `api_key_env` does not name a provider
API-key variable — this document is what the harness dispatches on, so it must
stay accurate. Merge Gateway and OpenRouter pins must also carry the route-terms fields.

## Fail-closed behavior

`medsynth-generate` checks this file before it emits a single teacher
request. A missing file, missing pin, incomplete field set, research-only
classification, unapproved family, or missing terms affirmation stops the
teacher lane. The procedural lane does not depend on this file and runs
fully offline.

The pipeline never calls a teacher API itself. Each request is written to
the `--teacher-responses` fulfillment directory as `<key>.request.json`;
an external harness dispatches it to the pinned endpoint and drops the
teacher's raw chat-completion JSON at `<key>.response.json`. Credentials
live only in that harness, under the env var named in the pin entry for
documentation — the pipeline never reads, holds, or logs them.

For a Merge Gateway pin the generated request sets
`include_routing_metadata: true`. The harness must preserve the response body.
If Merge returns vendor identity only in the `x-merge-vendor` header, the
harness writes `<sha256(response-json)> <vendor>` to `<key>.vendor.txt` beside
the raw response file. The hash binds the vendor attribution to that exact
response body, so replacing a response without refreshing the sidecar cannot
reuse stale routing metadata. When neither the body nor a matching sidecar
supplies vendor metadata the generated draft is dropped at the rights-envelope
gate rather than written.

For an OpenRouter pin the harness must send `X-OpenRouter-Metadata: enabled`
and preserve the response body. The pipeline reads the selected provider from
`openrouter_metadata.endpoints.available[]` where `selected` is true; if the
harness cannot preserve that body metadata, it must write the equivalent
`<sha256(response-json)> <vendor>` binding to `<key>.vendor.txt`. As with
Merge, missing vendor provenance drops the generated draft at the
rights-envelope gate.

For a Hermes route the harness must always export the session record as
`<key>.session.json`, regardless of how vendor metadata was supplied. The
record is the Hermes raw-trace JSON object: non-empty `run_id`, `session_id`,
`task_id`, and `raw_trace_id`; boolean `completed` and `partial`; and at least
one message object with non-empty string `role` plus string `content`. The
harness writes `<sha256(response-json)> <sha256(session-json)>` to
`<key>.session.txt`. That binding prevents a replacement response from reusing
a stale session. A missing, malformed, or unbound session record is treated as
absent and fails closed at the rights-envelope gate. Direct (non-Hermes)
teacher routes ignore these sidecars entirely.
