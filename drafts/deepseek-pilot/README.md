# DeepSeek teacher pilot configuration

This directory intentionally contains only the pilot target config. No teacher
output is committed here: fixture responses under
`crates/medsynth-generate/tests/fixtures/teacher/` exercise the parser and
batch machinery, but they are not DeepSeek output and must not be represented
as teacher-generated drafts.

To run the real pilot, fulfill the generated requests through the pinned
DeepSeek V4.1 Flash route in `docs/teachers.md` using the external
Hermes/Merge harness that owns the credentials:

```sh
cargo run -p medsynth-generate -- run \
  --config drafts/deepseek-pilot/config.json \
  --out drafts/deepseek-pilot \
  --teacher-responses path/to/fulfillments
```

The pipeline writes `<key>.request.json` files to the fulfillment directory.
The external harness dispatches them and writes the raw chat-completion JSON
as `<key>.response.json`. For the Hermes route it must also preserve
`<key>.session.json` and write the hash binding `<key>.session.txt` as
`<sha256(response-json)> <sha256(session-json)>`. Merge routing vendor
metadata must be present in the response body or in `<key>.vendor.txt` as
`<sha256(response-json)> <vendor>`.

Do not copy test fixture responses into the fulfillment directory for a real
pilot. Records admitted into the batch must carry truthful teacher, route,
vendor, response, and session provenance.
