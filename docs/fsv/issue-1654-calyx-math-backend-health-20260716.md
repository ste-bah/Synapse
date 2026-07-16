# Issue #1654 FSV - Calyx math backend selection in MCP health

Date: 2026-07-16
Agent: Codex
Issue: #1654

## Root Cause

Synapse already had a `math_backend` tuning enum, but it was configuration intent only. The daemon health path echoed `auto` and did not instantiate or persist the resolved Calyx Forge backend. There was no `synapse-calyx` factory that selected CUDA, fell back to CPU on CUDA initialization or probe failure, logged the selected backend, or exposed device/probe details through MCP health.

## Best-Practice Research Inputs

- Cargo feature defaults and optional dependency feature forwarding: https://doc.rust-lang.org/cargo/reference/features.html
- Rust runtime CPU feature detection for AVX-512: https://doc.rust-lang.org/std/macro.is_x86_feature_detected.html
- cudarc dynamic loading model: https://docs.rs/crate/cudarc/latest/source/src/lib.rs
- NVIDIA `CUDA_VISIBLE_DEVICES` behavior for hiding devices: https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/environment-variables.html

Applied decisions:

- `synapse-calyx` now enables `calyx-cuda` by default and forwards it to `calyx-forge/cuda`.
- `math_backend = "auto"` prefers `CudaBackend::new()` and records any CUDA failure before selecting CPU.
- `math_backend = "cpu"` is an explicit operator override.
- `math_backend = "cuda"` is rejected fail-closed because the issue only accepts `auto | cpu`.
- The health result is based on the resolved runtime status stored with the vault, not on config intent.

## Source Of Truth

- Trigger surface: real Codex `mcp__synapse.health` MCP tool against `http://127.0.0.1:7700/mcp`.
- Runtime process/socket: `Win32_Process` for `synapse-mcp.exe` and `Get-NetTCPConnection` on `127.0.0.1:7700`.
- Runtime health readback: authenticated `GET http://127.0.0.1:7700/health?detail=compact`.
- Vault artifacts: `vault.pid` and `vault-identity.json` under the active Calyx vault directory.
- Structured logs: `%LOCALAPPDATA%\synapse\logs\synapse.log.2026-07-16`.
- Invalid startup ledger: isolated `daemon-run-current.json` and `daemon-exit.jsonl`.
- GPU hardware readback: `nvidia-smi --query-gpu=name,memory.total,memory.free,driver_version --format=csv,noheader,nounits`.

## Minimal Synthetic Data

Startup probe vectors used by the real backend:

- Query: `[1.0, 0.0, 0.0]`
- Candidates: `[1.0, 0.0, 0.0]`, `[0.0, 1.0, 0.0]`, `[2.0, 0.0, 0.0]`
- Expected dot: `[1.0, 0.0, 2.0]`
- Expected cosine: `[1.0, 0.0, 1.0]`
- Expected l2 squared: `[0.0, 2.0, 1.0]`
- Top-k input: `[0.25, 1.5, -0.5, 1.5]`
- Expected top-k: `[(1, 1.5), (3, 1.5), (0, 0.25)]`
- Tolerance: `0.0001`

This is the smallest dataset that proves the selected backend executes each health-exposed operation class and preserves tie ordering for top-k.

## Happy Path - GPU Present

Before:

- Old daemon PID `61428` only reported `calyx_math_backend = "auto"` and had no resolved backend/device/probe health fields.
- Current installed binary hash before the final run was `F2D24E9756DF5CDB93946E2DA3DAF724CD033EDDCE3AB037C6582F4D4018A946`.
- Hardware SoT: `NVIDIA GeForce RTX 5090, 32607 MiB total, 27541 MiB free, driver 610.47`.

Trigger:

- Real MCP tool call: `mcp__synapse.health({ "detail": "compact" })`.
- Final restored daemon: PID `68704`, bind `127.0.0.1:7700`.
- MCP facade `tool_count = 40`, `tool_surface_sha256 = e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.

After separate SoT read:

- HTTP health PID `68704`, admin health `tool_count = 241`, `tool_surface_sha256 = 809d06d90b258a42daf82a92415754a105eb2422609704fff433735ee5840968`.
- Process: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe --mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\db-daemon`.
- `vault.pid` contains PID `68704`; `vault-identity.json` contains vault id `01KXKS2B1T5FPQ5722SJKNGSXR`.
- Health fields:
  - `calyx_math_backend = "cuda"`
  - `calyx_math_backend_requested = "auto"`
  - `calyx_math_cuda_compiled = true`
  - `calyx_math_device_name = "NVIDIA GeForce RTX 5090"`
  - `calyx_math_device_vram_mib = 32606`
  - `calyx_math_cpu_avx512_available = true`
  - `calyx_math_cpu_simd_path = "f32x16"`
  - `calyx_math_probe_status = "ok"`
  - Dot/cosine/l2/top-k exactly matched the expected synthetic data above.
- Log line `14827` records `SYNAPSE_CALYX_MATH_BACKEND_SELECTED` with `requested_backend="auto"`, `selected_backend="cuda"`, RTX 5090 device info, `fallback_code="none"`, and the same probe outputs.

## Edge 1 - Forced CPU Override

Before:

- Scheduled daemon PID `27640` was running on the normal db and vault.
- The scheduled task was stopped and PID `27640` was gracefully shut down through `/shutdown`, returning `ok=true`, `shutdown="requested"`.

Trigger:

- Synthetic config file: `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1654\cpu\calyx.toml`

```toml
[calyx]
math_backend = "cpu"
```

- Started real daemon PID `52228` on `127.0.0.1:7700`, isolated db `...\issue-1654\cpu\db`, isolated vault `...\issue-1654\cpu\vault`.
- Real MCP tool call: `mcp__synapse.health({ "detail": "compact" })`.

After separate SoT read:

- HTTP health PID `52228`.
- `vault.pid` contains PID `52228`; identity vault id `01KXN69H94FDT4FFZGZW0MRGFT`.
- Health fields:
  - `calyx_math_backend = "cpu"`
  - `calyx_math_backend_requested = "cpu"`
  - `calyx_math_device_name = "calyx-cpu"`
  - `calyx_math_cuda_compiled = true`
  - `calyx_math_fallback_code = null`
  - `calyx_math_probe_status = "ok"`
  - Dot/cosine/l2/top-k exactly matched the expected synthetic data.
- Log lines `14495` and `14496` record `SYNAPSE_CALYX_MATH_BACKEND_SELECTED` and `SYNAPSE_CALYX_VAULT_OPENED` with selected CPU and no fallback.

## Edge 2 - Structurally Invalid Override

Before:

- Forced-CPU daemon PID `52228` was listening on `127.0.0.1:7700`.
- It was gracefully shut down through `/shutdown`, returning `ok=true`, PID `52228`, `shutdown="requested"`.

Trigger:

- Synthetic config file: `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1654\invalid-config\calyx.toml`

```toml
[calyx]
math_backend = "cuda"
```

- Started real daemon PID `13248` with the invalid config.

After separate SoT read:

- Process exited with code `1`.
- No `synapse-mcp.exe` process remained.
- No listener remained on `127.0.0.1:7700`.
- `daemon-run-current.json` records PID `13248`, mode `http`, bind `127.0.0.1:7700`, `ended_reason = "top_level_error"`.
- `daemon-exit.jsonl` records cause `top_level_error` and:
  - `SYNAPSE_CALYX_CONFIG_INVALID: math_backend = "cuda" is not a supported Synapse override; use "auto" to prefer CUDA with CPU fallback or "cpu" to force CPU`
  - remediation: fix the `[calyx]` config or unset `SYNAPSE_CALYX_CONFIG`.
- This proves invalid config fails closed with a specific fix path instead of silently selecting another backend.

## Edge 3 - No CUDA Device Visible

Before:

- No `synapse-mcp.exe` process remained.
- No listener existed on `127.0.0.1:7700`.
- Scheduled task state was `Ready`.

Trigger:

- Synthetic config file: `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1654\missing-gpu\calyx.toml`

```toml
[calyx]
math_backend = "auto"
```

- Environment for the daemon process: `CUDA_VISIBLE_DEVICES=-1`.
- Started real daemon PID `15468` on `127.0.0.1:7700`, isolated db `...\issue-1654\missing-gpu\db`, isolated vault `...\issue-1654\missing-gpu\vault`.
- Real MCP tool call: `mcp__synapse.health({ "detail": "compact" })`.

After separate SoT read:

- HTTP health PID `15468`.
- `vault.pid` contains PID `15468`; identity vault id `01KXN6DMPVG37HPWT8Q7VW4QXT`.
- Health fields:
  - `calyx_math_backend = "cpu"`
  - `calyx_math_backend_requested = "auto"`
  - `calyx_math_device_name = "calyx-cpu"`
  - `calyx_math_fallback_code = "SYNAPSE_CALYX_MATH_CUDA_UNAVAILABLE"`
  - `calyx_math_fallback_source_code = "CALYX_FORGE_DEVICE_UNAVAILABLE"`
  - `calyx_math_fallback_error` includes `CUDA_ERROR_NO_DEVICE, "no CUDA-capable device is detected"`.
  - `calyx_math_probe_status = "ok"`
  - Dot/cosine/l2/top-k exactly matched the expected synthetic data.
- Log line `14686` records the warning `SYNAPSE_CALYX_MATH_CUDA_UNAVAILABLE`.
- Log line `14687` records selected CPU with `fallback_source_code="CALYX_FORGE_DEVICE_UNAVAILABLE"`.
- Log line `14688` records the vault opening and remaining functional under CPU fallback.

## Restore Readback

After edge cases:

- Temporary `SYNAPSE_CALYX_CONFIG`, `SYNAPSE_CALYX_VAULT_DIR`, and `CUDA_VISIBLE_DEVICES` were removed from the shell environment.
- Scheduled task `SynapseMcpDaemon` is `Running`.
- Normal daemon PID `68704` is listening on `127.0.0.1:7700`.
- Normal db path restored: `C:\Users\hotra\AppData\Local\synapse\db-daemon`.
- Normal vault path restored: `C:\Users\hotra\AppData\Roaming\synapse\vault`.
- Final MCP health selected CUDA with RTX 5090 and probe status `ok`.

## Supporting Structural Checks

These are compile/lint/format checks only, not FSV:

- `cargo fmt --all`
- `cargo check -p synapse-calyx`
- `cargo check -p synapse-mcp`
- `cargo check -p synapse-calyx --no-default-features`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets`

## Follow-Up Filed

During repo-built daemon setup, Chrome bridge repair failed to find the extension reload button through UI Automation. That is outside #1654 and was filed as #1707.
