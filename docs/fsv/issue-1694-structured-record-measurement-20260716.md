# Issue 1694 - structured-record measurement pipeline

Date: 2026-07-16
Issue: https://github.com/ChrisRoyse/Synapse/issues/1694

## Root Cause

`calyx-registry` could measure raw `Input` bytes through panel/lens paths, but it had no general
structured-record boundary. Synapse timeline rows are typed JSON records, so callers had to choose
between flattening the whole row into bytes or hand-routing fields outside Calyx. That left no
fail-closed contract for field paths, typed extraction, optional absence, scalar preservation, or
metadata routing.

A related physical-storage gap was exposed while implementing the acceptance readback: Aster declared
`ColumnFamily::Scalars` as `(ScalarId, CxId) -> f64`, but normal constellation commit staging only
wrote scalars inside the Base row. The Scalar CF therefore could not be used as physical proof that a
structured numeric field had been materialized.

Manual FSV also exposed dependency drift: a standalone consumer of `calyx-registry` resolved
`fastembed` 5.17.3, which pulled a newer Candle stack and broke the Qwen3 compile boundary. The
workspace lock masked this until the library was used from outside the workspace.

## Research Used

- Exa and web search reviewed RFC 8785 JSON Canonicalization Scheme. It requires deterministic JSON
  object property sorting, recursive sorting, unchanged array order, no emitted whitespace, and UTF-8
  output: https://datatracker.ietf.org/doc/html/rfc8785
- Exa and web search reviewed RFC 6901 JSON Pointer. It defines slash-prefixed reference tokens,
  `~0` / `~1` escaping, and failed evaluation for invalid syntax or nonexistent values:
  https://datatracker.ietf.org/doc/html/rfc6901

Implementation applies those ideas as a deterministic compact JSON encoder with recursive UTF-16 key
ordering and strict JSON Pointer validation, while keeping Calyx-specific missing optional fields as
explicit `SlotVector::Absent` instead of treating every missing pointer as a hard error.

## Fix

- Added `calyx-registry::structured_record`:
  - `StructuredRecordSchema`, `StructuredFieldSchema`, typed field expectations, slot/scalar/metadata
    routes, and batch measurement output.
  - `canonical_json_bytes` with recursive object-key sorting and compact UTF-8 JSON output.
  - Fail-closed schema validation for empty domains, panel version `0`, invalid JSON pointers,
    duplicate slot/scalar/metadata destinations, empty route keys, and scalar routes on non-numeric
    fields.
  - Fail-closed record validation for non-object records, required missing fields, type mismatches,
    non-finite numbers, and integer scalar values outside exact f64 range.
  - Optional missing fields write `SlotVector::Absent { reason: Error("structured_field_missing:<path>") }`.
  - String lens inputs use raw UTF-8 string bytes; non-string lens inputs use canonical JSON bytes.
- Added deterministic Aster scalar-row materialization:
  - `scalar_id_for_key(name)` derives a stable `ScalarId` from the scalar key.
  - Constellation staging writes every scalar to `ColumnFamily::Scalars` as big-endian f64 bits.
  - Per-constellation scalar-id collisions fail closed before commit.
- Exact-pinned `fastembed` to `=5.16.0`, the version compatible with the current Candle stack.

## Source Of Truth

- Runtime/client SoT: real `mcp__synapse` client, daemon `health`, tool list, daemon PID, and bind.
- Manual trigger SoT: real `mcp__synapse.shell` operation records under Synapse shell job/session
  storage.
- Calyx storage SoT: durable Aster vault at
  `C:\Users\hotra\AppData\Local\Temp\synapse1694-manual-fsv-vault`.
- Physical row SoTs:
  - Base CF: `cf/base`, key `CxId`.
  - Slot CFs: `cf/slot_01` through `cf/slot_04`, key `CxId`.
  - Scalars CF: `cf/scalars`, key `(scalar_id_for_key(name), CxId)`, value big-endian f64 bits.

## Runtime Preconditions

Real MCP client surface was used.

- `mcp__synapse.health` returned `ok=true`.
- Daemon PID: `75204`.
- Bind: `127.0.0.1:7700`.
- Tool count: `40`.
- Tool surface SHA-256: `e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.
- Tool names included `health`, `shell`, `storage`, and `timeline`.
- Calyx vault subsystem was open and healthy.

Real captured row source:

- `mcp__synapse.timeline operation=get limit=5` read CF_TIMELINE rows.
- The happy-path batch measured one captured `focus_change` row:
  - actor `human`
  - app `chrome.exe`
  - title `Leapable - Vaults - Google Chrome`
  - pid `25400`
  - ts_ns `1781230338259628800`

## Manual FSV Trigger

Trigger was a real `mcp__synapse.shell operation=run` that created a temporary operator program
outside the repo and ran it with:

```text
cargo run --quiet --manifest-path %TEMP%\synapse1694-manual-fsv\Cargo.toml
```

The program used local path dependencies on the modified Calyx crates, opened a real durable Aster
vault, registered four frozen algorithmic lenses, declared a `synapse.timeline.seed` schema, measured
records through `measure_structured_record_batch`, and persisted accepted records through real
`ingest_at`.

Initial SoT:

```json
{"base_rows":0,"scalar_rows":0,"slot_1_rows":0,"slot_2_rows":0,"slot_3_rows":0,"slot_4_rows":0,"snapshot":0}
```

Happy-path trigger:

```text
TRIGGER ingest index=0 canonical_sha256=cada7ca2cb7b5244e562cb30d29122f8a87e691333a6776105560fc16d762771 cx_id=371e122f2e8ea2edf13db3324d2bf967
TRIGGER ingest index=1 canonical_sha256=33b9b59e41a3e328980f35878b2a4ba22396ddfa8c1f9b077c4aa49d32974883 cx_id=637a7336ba1dbab96f3580aa055e1228
```

Happy-path after-state:

```json
{"base_rows":2,"scalar_rows":4,"slot_1_rows":2,"slot_2_rows":2,"slot_3_rows":2,"slot_4_rows":2,"snapshot":2}
```

Happy-path physical readback:

- Captured timeline row `371e122f2e8ea2edf13db3324d2bf967`:
  - Base row present.
  - Slot 1 dense present, slot 2 dense present, slot 3 dense present, slot 4 sparse present.
  - Metadata contained `actor=human`, `app=chrome.exe`, `kind=focus_change`, and title.
  - Scalar CF `pid=25400.0` matched Base scalar `pid=25400.0`.
  - Scalar CF `event_time_secs=1784200000.0` matched Base scalar.
- Synthetic happy row `637a7336ba1dbab96f3580aa055e1228`:
  - Base row present.
  - Slot 1 dense present, slot 2 dense present, slot 3 dense present, slot 4 sparse present.
  - Scalar CF `pid=4242.0` matched Base scalar.
  - Scalar CF `event_time_secs=1784200001.0` matched Base scalar.

## Edge Cases

### 1. Empty Batch

Before:

```json
{"base_rows":2,"scalar_rows":4,"slot_1_rows":2,"slot_2_rows":2,"slot_3_rows":2,"slot_4_rows":2,"snapshot":2}
```

Trigger result:

```text
EDGE empty.trigger measured_records=0
```

After:

```json
{"base_rows":2,"scalar_rows":4,"slot_1_rows":2,"slot_2_rows":2,"slot_3_rows":2,"slot_4_rows":2,"snapshot":2}
```

Outcome: no write occurred.

### 2. Missing Optional Fields

Input omitted `/app` and `/payload/title` but included the maximum exact JSON integer scalar
`pid=9007199254740991`.

Before:

```json
{"base_rows":2,"scalar_rows":4,"slot_1_rows":2,"slot_2_rows":2,"slot_3_rows":2,"slot_4_rows":2,"snapshot":2}
```

Trigger:

```text
TRIGGER ingest index=0 canonical_sha256=64145d8d7e3fae0dddee214fb65c68efeb8aace4f92f580cab960bcafd27562c cx_id=d81101f9fb2d9d04d71bfedba9b8f657
```

After:

```json
{"base_rows":3,"scalar_rows":6,"slot_1_rows":3,"slot_2_rows":3,"slot_3_rows":3,"slot_4_rows":3,"snapshot":3}
```

Physical readback:

- Base row present.
- Slot 1 dense present and slot 2 dense present.
- Slot 3 raw CF row present with hydrated kind `absent` and reason
  `structured_field_missing:/app`.
- Slot 4 raw CF row present with hydrated kind `absent` and reason
  `structured_field_missing:/payload/title`.
- Scalar CF `pid=9007199254740991.0` matched Base scalar.

Outcome: missing optional fields became explicit Absent rows, never zero vectors.

### 3. Structurally Invalid Required Field

Input set `/kind` to a number.

Before:

```json
{"base_rows":3,"scalar_rows":6,"slot_1_rows":3,"slot_2_rows":3,"slot_3_rows":3,"slot_4_rows":3,"snapshot":3}
```

Trigger result:

```text
EDGE invalid_type.error code=CALYX_STRUCTURED_FIELD_TYPE_MISMATCH message=record 0 field /kind expected String, got number
```

After:

```json
{"base_rows":3,"scalar_rows":6,"slot_1_rows":3,"slot_2_rows":3,"slot_3_rows":3,"slot_4_rows":3,"snapshot":3}
```

Outcome: structured error, no partial write.

### 4. Unsafe Integer Scalar

Input set `/payload/pid` to `9007199254740992`, just above the exact f64 JSON integer boundary.

Before:

```json
{"base_rows":3,"scalar_rows":6,"slot_1_rows":3,"slot_2_rows":3,"slot_3_rows":3,"slot_4_rows":3,"snapshot":3}
```

Trigger result:

```text
EDGE unsafe_scalar.error code=CALYX_STRUCTURED_FIELD_TYPE_MISMATCH message=record 0 field /payload/pid integer 9007199254740992 exceeds exact f64 JSON scalar range
```

After:

```json
{"base_rows":3,"scalar_rows":6,"slot_1_rows":3,"slot_2_rows":3,"slot_3_rows":3,"slot_4_rows":3,"snapshot":3}
```

Outcome: structured error, no partial write.

## Independent Read-Only Readback

A second real `mcp__synapse.shell operation=run` opened the same vault in a separate process and did
only read operations.

Read-only counts:

```json
{"base_rows":3,"scalar_rows":6,"slot_1_rows":3,"slot_2_rows":3,"slot_3_rows":3,"slot_4_rows":3}
```

Read-only record proof:

- `371e122f2e8ea2edf13db3324d2bf967`
  - Base raw row present, length `558`.
  - Slot raw rows present: slot 1 length `133`, slot 2 length `69`, slot 3 length `261`, slot 4 length `49`.
  - Scalar raw rows present: `pid` length `8`, value `25400.0`; `event_time_secs` length `8`, value `1784200000.0`.
- `637a7336ba1dbab96f3580aa055e1228`
  - Base raw row present, length `553`.
  - Slot raw rows present: slot 1 length `133`, slot 2 length `69`, slot 3 length `261`, slot 4 length `49`.
  - Scalar raw rows present: `pid` length `8`, value `4242.0`; `event_time_secs` length `8`, value `1784200001.0`.
- `d81101f9fb2d9d04d71bfedba9b8f657`
  - Base raw row present, length `492`.
  - Slot raw rows present: slot 1 length `133`, slot 2 length `69`, slot 3 length `35`, slot 4 length `45`.
  - Slot 3 hydrated as absent with `structured_field_missing:/app`.
  - Slot 4 hydrated as absent with `structured_field_missing:/payload/title`.
  - Scalar raw rows present: `pid` length `8`, value `9007199254740991.0`; `event_time_secs` length `8`, value `1784200001.0`.

## Structural Checks

- `cargo fmt --manifest-path calyx\Cargo.toml --all --check`: passed.
- `cargo check --manifest-path calyx\Cargo.toml -p calyx-registry -p calyx-aster`: passed.
- `cargo clippy --manifest-path calyx\Cargo.toml -p calyx-registry -p calyx-aster --all-targets`: passed.
- `cargo check --manifest-path calyx\Cargo.toml --workspace`: passed.
- `cargo clippy --manifest-path calyx\Cargo.toml --workspace --all-targets`: passed.

No automated tests, benches, FSV scripts, FSV harnesses, or repo examples were added or run.
