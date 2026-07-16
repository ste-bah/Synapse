# Issue #1696 Calyx bridge manual FSV - 2026-07-16

Issue: https://github.com/ChrisRoyse/Synapse/issues/1696

## Scope

This change bridges the absorbed Calyx crates into Synapse runtime policy:

- Calyx `CALYX_*` errors surfaced through `synapse-calyx` are mapped to
  `SYNAPSE_CALYX_*` codes, with the upstream code retained as
  `source_code`.
- Optional `[calyx]` TOML config exposes the handbook knobs and validates
  fail-closed before daemon serving.
- Calyx vault open uses an injected Synapse-owned clock so fixed-clock
  runtime state can be read back from health.
- MCP health reports the effective Calyx tuning so config changes have a
  physical runtime Source of Truth.

The current public 40-tool MCP surface has no post-startup Calyx row
read/write tool. A corrupted Calyx vault must fail daemon startup closed rather
than serve a `tools/call`; #1656 is the existing issue that will add the real
Calyx-backed daemon data path for later row-level MCP-triggered FSV.

## Root Cause

The root issue was a missing bridge layer. `SynapseCalyxError::from_calyx`
copied upstream `error.code` directly, so raw `CALYX_*` strings could escape
into Synapse health/startup surfaces. Synapse also had no runtime `[calyx]`
config parser, no health readback of effective tuning, and the wrapper always
opened `AsterVault` with `SystemClock`.

## Research Inputs

- Exa MCP and native web research read Serde container attributes:
  https://serde.rs/container-attrs.html
- Rust error guidance:
  https://doc.rust-lang.org/std/error/index.html
- `thiserror` source/error wrapping guidance:
  https://docs.rs/thiserror/latest/thiserror/
- Clock injection precedent:
  https://docs.rs/clocks/latest/clocks/

Design conclusions applied:

- Use `serde(deny_unknown_fields)` so config typos fail closed instead of
  being ignored.
- Represent expected runtime/config faults as structured `Result` errors, not
  panics.
- Preserve source error identity across layers.
- Inject time through a clock abstraction for deterministic Calyx-dependent
  behavior.

## Fix

- Added `crates/synapse-calyx/src/error_bridge.rs` with an explicit mapping
  for the 39 `calyx_core::CALYX_ERROR_CODES`, a compile-time count pin, and
  runtime drift validation.
- Added `SynapseCalyxTuningConfig` defaults/validation and optional
  `--calyx-config` / `SYNAPSE_CALYX_CONFIG` support.
- Added `SynapseCalyxClock` and changed vault open to
  `AsterVault::open_with_clock`.
- Extended Calyx health fields with effective tuning, clock mode, RNG seed,
  and `last_calyx_error_code`.
- Updated async-vault expired-lease handling to match the retained
  Calyx `source_code` instead of the mapped Synapse code.
- Documented the `[calyx]` section and defaults in
  `docs/systemdocs/03_configuration.md`.

## Source Of Truth

Runtime/daemon SoTs:

- Process table for `synapse-mcp.exe`.
- Socket table for `127.0.0.1:7700`.
- `C:\Users\hotra\AppData\Local\synapse\db-daemon\daemon-run-current.json`.
- Wired Codex MCP client `mcp__synapse.setup` and `mcp__synapse.health`.
- MCP daemon tool ledger
  `C:\Users\hotra\AppData\Local\synapse\db-daemon\daemon-tool-events.jsonl`.
- Calyx vault PID sidecar:
  `C:\Users\hotra\AppData\Roaming\synapse\vault\vault.pid`.

Synthetic config/vault SoTs used during manual FSV:

- `C:\Users\hotra\AppData\Local\Temp\synapse-1696-fsv\valid-boundary-calyx.toml`
  SHA256 `A1A381CD3FA2ABCC8EE56A694D2BB4A6CA02F3D48180B9C37A2AB177440B1E0B`.
- Isolated edge databases under
  `C:\Users\hotra\AppData\Local\Temp\synapse-1696-fsv\db-*`.
- Isolated edge vaults under
  `C:\Users\hotra\AppData\Local\Temp\synapse-1696-fsv\vault-*`.

Valid config contents:

```toml
[calyx]
bit_floor_bits = 0.125
correlation_ceiling = 0.5
guard_far_identity = 0.005
guard_far_content = 0.02
guard_far_stylistic = 0.04
guard_cold_start_tau = 0.8
kernel_fraction = 1.0
kernel_recall_gate = 0.97
fusion_k = 1
temporal_boost_min = 0.10
temporal_boost_max = 0.10
vram_budget_bytes = 1073741824
math_backend = "cpu"
clock_mode = "fixed"
fixed_clock_unix_ms = 1720000000123
rng_seed = 1696
```

## MCP Precondition

Before replacing the daemon, the wired client was live:

- `mcp__synapse.setup(operation=status)` read PID `46828`, bind
  `127.0.0.1:7700`, token file present, and Codex config pointing at bearer
  env `SYNAPSE_BEARER_TOKEN`.
- `mcp__synapse.health(detail=compact)` returned `ok=true`, `pid=46828`,
  `tool_count=40`, and a valid 40-tool schema surface.
- OS SoT readback matched PID `46828`,
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, and listener ownership on
  `127.0.0.1:7700`.

Then I stopped only verified PID `46828` and started the repo-built release
binary from `C:\code\Synapse\target\release\synapse-mcp.exe`.

## Happy Path - Runtime Config Readback

Expected: valid boundary `[calyx]` config starts the repo-built daemon, opens a
synthetic vault, and the real MCP health tool reports the effective tuning.

Before trigger:

- No listener on `127.0.0.1:7700`.
- Config file hash:
  `A1A381CD3FA2ABCC8EE56A694D2BB4A6CA02F3D48180B9C37A2AB177440B1E0B`.

Trigger:

- Started `target\release\synapse-mcp.exe` on `127.0.0.1:7700` with:
  `--calyx-config <valid-boundary-calyx.toml>` and
  `--calyx-vault-dir <vault-valid-boundary>`.
- Called real wired `mcp__synapse.setup(operation=status)`.
- Called real wired `mcp__synapse.health(detail=compact)`.

After readback:

- Process PID: `51152`.
- Executable:
  `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Listener: `127.0.0.1:7700`, owning process `51152`.
- Run file PID: `51152`, mode `http`, bind `127.0.0.1:7700`.
- Vault PID sidecar:
  `{"exe":"C:\\code\\Synapse\\target\\release\\synapse-mcp.exe","pid":51152,"schema_version":1}`.
- MCP setup readback: PID `51152`, token file present, Codex config valid.
- MCP health readback:
  - `tool_count=40`
  - `calyx_vault.status=ok`
  - `calyx_vault_open=true`
  - `calyx_vault_path=<vault-valid-boundary>`
  - `calyx_bit_floor_bits=0.125`
  - `calyx_correlation_ceiling=0.5`
  - `calyx_kernel_fraction=1.0`
  - `calyx_kernel_recall_gate=0.9700000286102295`
  - `calyx_fusion_k=1`
  - `calyx_temporal_boost_min=0.10000000149011612`
  - `calyx_temporal_boost_max=0.10000000149011612`
  - `calyx_vram_budget_bytes=1073741824`
  - `calyx_math_backend=cpu`
  - `calyx_clock_mode=fixed`
  - `calyx_fixed_clock_unix_ms=1720000000123`
  - `calyx_rng_seed=1696`
- Daemon tool ledger recorded the real MCP `health` call as `status=ok`,
  `tool=health`, profile `normal_agent`, and the same run id as PID `51152`.

## Edge 1 - Empty Config

Expected: empty config fails startup closed, leaves no listener, and records a
structured remediation.

Before:

- Port `7791`: no socket rows.
- Config text: empty/whitespace.

Trigger:

- Started repo-built daemon on isolated port `7791`, isolated db, isolated
  vault, and `--calyx-config empty.toml`.

After:

- Process PID `56040` exited with code `1`.
- Port `7791`: no listener.
- `daemon-exit.jsonl` contained:
  `SYNAPSE_CALYX_CONFIG_PARSE_FAILED`, `missing field 'calyx'`, and
  remediation `fix the [calyx] configuration file or unset SYNAPSE_CALYX_CONFIG
  to use handbook defaults`.

## Edge 2 - Unknown Config Key

Expected: typo keys fail closed through serde unknown-field validation.

Before:

- Port `7792`: no socket rows.
- Config text:
  `bit_floor_bits = 0.1` and `unknown_knob = 1`.

Trigger:

- Started repo-built daemon on isolated port `7792`, isolated db, isolated
  vault, and `--calyx-config unknown-key.toml`.

After:

- Process PID `55048` exited with code `1`.
- Port `7792`: no listener.
- `daemon-exit.jsonl` contained:
  `SYNAPSE_CALYX_CONFIG_PARSE_FAILED`, `unknown field 'unknown_knob'`, the
  allowed key list, and the config remediation.

## Edge 3 - Boundary Invalid Kernel Fraction

Expected: `kernel_fraction = 0.0` fails validation even though it parses.

Before:

- Port `7793`: no socket rows.
- Config text: `[calyx] kernel_fraction = 0.0`.

Trigger:

- Started repo-built daemon on isolated port `7793`, isolated db, isolated
  vault, and `--calyx-config zero-kernel.toml`.

After:

- Process PID `14888` exited with code `1`.
- Port `7793`: no listener.
- `daemon-exit.jsonl` contained:
  `SYNAPSE_CALYX_CONFIG_INVALID: kernel_fraction must be greater than 0.0`
  and the config remediation.

## Edge 4 - Fixed Clock Missing Timestamp

Expected: `clock_mode = "fixed"` without `fixed_clock_unix_ms` fails closed.

Before:

- Port `7794`: no socket rows.
- Config text: `[calyx] clock_mode = "fixed"`.

Trigger:

- Started repo-built daemon on isolated port `7794`, isolated db, isolated
  vault, and `--calyx-config fixed-missing-timestamp.toml`.

After:

- Process PID `71408` exited with code `1`.
- Port `7794`: no listener.
- `daemon-exit.jsonl` contained:
  `SYNAPSE_CALYX_CONFIG_INVALID: clock_mode = "fixed" requires
  fixed_clock_unix_ms` and the config remediation.

## Edge 5 - Calyx-Originated Error Mapping

Expected: a physical Calyx manifest failure maps to a Synapse error code and
retains the upstream `CALYX_*` code.

Before:

- Port `7795`: no socket rows.
- Vault directory contained `CURRENT` with text `not-a-manifest`.

Trigger:

- Started repo-built daemon on isolated port `7795`, isolated db, valid config,
  and the corrupt vault directory.

After:

- Process PID `27008` exited with code `1`.
- Port `7795`: no listener.
- Vault directory still contained the corrupt `CURRENT`; Synapse-created
  `vault-identity.json` and `vault.lock` remained for diagnosis; no live
  process held the port.
- `daemon-exit.jsonl` contained the mapped structured error:
  `SYNAPSE_CALYX_ASTER_CORRUPT_SHARD: open durable Calyx Aster vault: CURRENT
  does not point at immutable manifest file; source_code=CALYX_ASTER_CORRUPT_SHARD;
  remediation=restore from restic/snapshot`.

This is the fail-closed startup surface, not a served MCP `tools/call`: a
daemon with a corrupt configured Calyx vault must not serve the MCP API.

## Final Default Daemon Readback

After edge runs, I stopped only verified PID `51152` and restarted the
repo-built daemon without synthetic config or vault flags.

Final SoT:

- PID `76992`.
- Executable:
  `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Command line:
  `--mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\db-daemon --profile-dir C:\Users\hotra\.cargo\bin\profiles --log-level info`.
- Run file PID `76992`, bind `127.0.0.1:7700`, `ended_reason=null`.
- Default vault PID sidecar:
  `{"exe":"C:\\code\\Synapse\\target\\release\\synapse-mcp.exe","pid":76992,"schema_version":1}`.
- `mcp__synapse.setup(operation=status)` returned PID `76992` and the same
  bearer-configured Codex transport.
- `mcp__synapse.health(detail=compact)` returned:
  - `ok=true`
  - `tool_count=40`
  - `calyx_vault.status=ok`
  - `calyx_vault_open=true`
  - `calyx_vault_path=C:\Users\hotra\AppData\Roaming\synapse\vault`
  - `calyx_bit_floor_bits=0.05000000074505806`
  - `calyx_correlation_ceiling=0.6000000238418579`
  - `calyx_kernel_fraction=0.009999999776482582`
  - `calyx_kernel_recall_gate=0.949999988079071`
  - `calyx_fusion_k=60`
  - `calyx_temporal_boost_min=0.0`
  - `calyx_temporal_boost_max=0.10000000149011612`
  - `calyx_vram_budget_bytes=12884901888`
  - `calyx_math_backend=auto`
  - `calyx_clock_mode=system`
  - `calyx_rng_seed=6491761763268826774`

## Structural Checks

Supporting structural checks only; these are not FSV:

- `cargo check -p synapse-calyx` passed.
- `cargo check -p synapse-core` passed after removing invalid `Eq` derives
  from health structs that now carry `f32`.
- `cargo check -p synapse-mcp` passed.
- `cargo fmt --all --check` passed.
- `cargo clippy --workspace --all-targets` passed with only the existing
  dependency build warning:
  `calyx-forge@0.1.0: cuda feature not enabled, skipping kernel compilation`.
- `cargo build --release -p synapse-mcp` passed and was used only to run the
  real daemon for final manual FSV.

No tests, benchmarks, GitHub Actions, CI, automated FSV scripts, or FSV
harnesses were run.

## Result

Issue #1696 is verified for the bridge layer: Calyx errors are mapped into
Synapse codes with upstream source retention, Calyx config loads and rejects
bad state fail-closed at startup, fixed/system clock policy is read back in
health, and the real wired MCP client observes the effective runtime defaults
and custom config values from the repo-built daemon.
