# DeepSeek teacher pilot configuration

This directory intentionally contains only the pilot target config. No teacher
output is committed here: fixture responses under
`crates/medsynth-generate/tests/fixtures/teacher/` exercise the parser and
batch machinery, but they are not DeepSeek output and must not be represented
as teacher-generated drafts.

To run the real pilot, fulfill the generated requests through the pinned
DeepSeek V4.1 Flash route in `docs/teachers.md` using the external
Hermes/OpenRouter harness that owns the credentials:

```sh
cargo run -p medsynth-generate -- run \
  --config drafts/deepseek-pilot/config.json \
  --out drafts/deepseek-pilot \
  --teacher-responses path/to/fulfillments
```

The pipeline writes `<key>.request.json` files to the fulfillment directory. Each request restricts OpenRouter to the `deepseek` provider with fallbacks disabled.
The external harness dispatches them and writes the raw chat-completion JSON
as `<key>.response.json`. For the Hermes route it must also preserve
`<key>.session.json` and write the hash binding `<key>.session.txt` as
`<sha256(response-json)> <sha256(session-json)>`. OpenRouter routing vendor
metadata must be present in the response body or in `<key>.vendor.txt` as
`<sha256(response-json)> <vendor>`. The harness must send
`X-OpenRouter-Metadata: enabled` so the response carries
`openrouter_metadata.endpoints.available[]` with the selected provider.

The first run is expected to exit nonzero when the fulfillment directory is
empty: its purpose is to emit the request files. After the harness has written
the responses and sidecars, rerun the same command. The replay writes the same
deterministic request bodies, finds the matching `<key>.response.json` files,
validates the response/session bindings, and writes the draft shards and
manifest.

Do not copy test fixture responses into the fulfillment directory for a real
pilot. Records admitted into the batch must carry truthful teacher, route,
vendor, response, and session provenance.
