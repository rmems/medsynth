# Teacher pin registry

**Status:** binding for the teacher lane of the generation pipeline (#8).
**Snapshot taken:** 2026-11-21, before any teacher output was generated.
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

Terms pages were read on **2026-11-21**. Terms can change; re-snapshot and
re-review before a later run. The exact clause relied on is quoted in each
entry, so the basis is auditable without re-fetching the page.

| Teacher | Terms document | Version / date | URL |
| --- | --- | --- | --- |
| DeepSeek V4.1 Flash | DeepSeek Open Platform Terms of Service | Release 2026-04-22; effective 2026-04-29 | <https://cdn.deepseek.com/policies/en-US/deepseek-open-platform-terms-of-service.html> |
| NVIDIA Nemotron 3 Ultra | NVIDIA Nemotron Open Model License | v. December 15, 2025 | <https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-nemotron-open-model-license/> |

## Teacher pins

### Pin: `deepseek-v4.1-flash`

- **pin_id:** `deepseek-v4.1-flash`
- **provider:** `DeepSeek Open Platform`
- **family:** `deepseek`
- **variant:** `DeepSeek-V4.1-Flash-1017`
- **model_id:** `deepseek-flash`
- **endpoint:** `https://api.deepseek.com/chat/completions`
- **api_key_env:** `DEEPSEEK_API_KEY`
- **lane:** `teacher`
- **terms_document:** `DeepSeek Open Platform Terms of Service`
- **terms_version:** `release 2026-04-22; effective 2026-04-29`
- **terms_date:** `2026-04-29`
- **terms_url:** `https://cdn.deepseek.com/policies/en-US/deepseek-open-platform-terms-of-service.html`
- **terms_clause:** `§4.2(3): "You may apply the Inputs and Outputs of the Services to a wide range of use cases, including personal use, academic research, derivative product development, training other models (such as model distillation), etc."`
- **permits_distillation:** `yes`
- **permits_synthetic_data:** `yes`
- **snapshot_date:** `2026-11-21`
- **notes:** `Variant as updated 2026-10-17 (DeepSeek-V4.1-Flash-1017); API model id deepseek-flash. Outputs are assigned to the developer under §4.2(2) and may be used for model distillation under §4.2(3).`

### Pin: `nemotron-3-ultra`

- **pin_id:** `nemotron-3-ultra`
- **provider:** `NVIDIA NIM (build.nvidia.com)`
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
- **snapshot_date:** `2026-11-21`
- **notes:** `Nemotron 3 family, Ultra 550B A55B NVFP4 variant served by NVIDIA NIM. The license does not claim ownership of outputs and grants derivative-work rights, so outputs may be used to build synthetic training data.`

## Never teachers

These are recorded here so the exclusion is visible, not because they are
pins. Their outputs are research-only for this repository and are never used
as teacher drafts:

- **Grok (xAI)** — research-only. Not an approved teacher family.
- **Claude (Anthropic)** — research-only. Not an approved teacher family.

A pin entry whose `lane` is not `teacher`, or whose `family` is not `deepseek`
or `nvidia-nemotron`, is rejected by the gate even if its terms look
permissive. The gate also rejects an entry that omits any required field or
that does not affirm both `permits_distillation` and
`permits_synthetic_data`.

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
