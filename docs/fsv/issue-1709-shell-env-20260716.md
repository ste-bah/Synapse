# Issue #1709 FSV - Shell Child Build Environment

Date: 2026-07-16
Issue: https://github.com/ChrisRoyse/Synapse/issues/1709

## Root Cause

`act_run_shell` already called `env_clear()` and rebuilt a bounded child environment, but its
allowlist did not include CUDA/nvcc build variables. Adding those variables naively would still
leave a stale-daemon bug: non-PATH registry values only filled absent keys, so a long-lived
`synapse-mcp.exe` process could keep an old `CUDA_PATH` or `NVCC_*` value and pass it to future
shell children even after the durable Windows environment changed.

## Research Used

- Microsoft Environment Variables: child processes inherit parent environment by default; callers
  can pass an explicit environment block to `CreateProcess`; system environment registry updates
  require `WM_SETTINGCHANGE` for applications to pick up changes.
  https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables
- Microsoft Changing Environment Variables: one process cannot directly change another non-child
  process environment; changing child environment happens at child creation.
  https://learn.microsoft.com/en-us/windows/win32/procthread/changing-environment-variables
- Rust `std::process::Command`: child processes inherit environment by default, and
  `Command::env_clear` prevents inheriting parent variables.
  https://doc.rust-lang.org/std/process/struct.Command.html

## Fix

- Added CUDA build environment keys to the child allowlist:
  `CUDA_PATH`, `CUDA_PATH_V13_3`, `FORGE_CUDA_CCBIN`, `NVCC_CCBIN`,
  `NVCC_APPEND_FLAGS`, and `NVCC_PREPEND_FLAGS`.
- For those keys, durable Windows HKCU/HKLM registry values override the daemon process snapshot.
- `shell_child_environment` now builds one explicit child environment map for both inline and
  durable shell paths, then `env_clear()` + explicit mappings are applied to the spawned child.
- CUDA-relevant commands (`cargo`, `nvcc`, or shell wrappers that invoke them) fail closed before
  spawn when durable `CUDA_PATH` is missing, empty, or does not contain `bin\nvcc.exe`.
- Durable job status includes `environment_diagnostics` with:
  `SYNAPSE_SHELL_ENV_DAEMON_STALE`, `SYNAPSE_SHELL_ENV_DURABLE_MISSING`, or
  `SYNAPSE_SHELL_ENV_DURABLE_INVALID`.

## Source Of Truth

- Runtime/client SoT: real `mcp__synapse` tool surface, `health`, daemon PID, and TCP listener.
- Durable environment SoT: `HKCU\Environment` and
  `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`.
- Shell job SoT:
  `%LOCALAPPDATA%\synapse\shell-jobs\jobs\<job_id>\status.json`,
  `stdout.log`, and `stderr.log`.
- Final host hygiene SoT: scheduled task `SynapseMcpDaemon`, process table, socket table, and
  environment broadcast return.

## Runtime Preconditions

Real MCP client surface was used for all shell triggers.

- Initial daemon for most FSV: PID `15332`, bind `127.0.0.1:7700`.
- Installed binary hash:
  `4F668DE5CF342C0B7347C002ACEEB60011912AFFDF054B8ADE7304DA44FD7912`.
- Tool count: `40`; tool surface SHA-256:
  `f457205fa2fb76db06c1157808fd54f417ace660f3398df74f80bb3c21383ffe`.
- Tool names included `shell`.
- Process/socket readback showed `synapse-mcp.exe` PID `15332` owned the listener.

## Happy Path

Before:

```json
{"Process":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","User":null,"Machine":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","GetCommandNvcc":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3\\bin\\nvcc.exe","NvccExists":true}
```

Trigger: real `mcp__synapse.shell` with `operation=start`, job
`issue1709-happy-20260716T1510`, command `powershell.exe`, checking
`$env:CUDA_PATH` and `Get-Command nvcc.exe`.

After separate file readback:

```json
{"job_id":"issue1709-happy-20260716T1510","status":"ok","exit_code":0,"stdout_ok":true,"stdout_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","stdout_nvcc_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3\\bin\\nvcc.exe","stderr_bytes":0}
```

## Edge 1 - Invalid Durable CUDA Root

Before trigger, HKCU was set to a synthetic invalid root:

```json
{"user_cuda_path":"C:\\__synapse_issue1709_invalid_cuda_root__","user_path_exists":false,"expected_nvcc_exists":false,"machine_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","process_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3"}
```

Trigger: real `mcp__synapse.shell` with `operation=start`, job
`issue1709-invalid-durable-20260716T1530`, command `cargo --version`.

After separate file readback:

```json
{"job_id":"issue1709-invalid-durable-20260716T1530","status":"spawn_failed","pid":null,"exit_code":null,"error_code":"ACTION_TARGET_INVALID","diagnostic_code":"SYNAPSE_SHELL_ENV_DURABLE_INVALID","process_value":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","durable_value":"C:\\__synapse_issue1709_invalid_cuda_root__","effective_value":"C:\\__synapse_issue1709_invalid_cuda_root__","validation_path":"C:\\__synapse_issue1709_invalid_cuda_root__\\bin\\nvcc.exe","stdout_bytes":0,"stderr_bytes":0}
```

## Edge 2 - Explicit Per-Job CUDA Override

With HKCU still invalid, the job explicitly supplied a valid `CUDA_PATH`.

After separate file readback:

```json
{"job_id":"issue1709-explicit-override-20260716T1532","status":"ok","exit_code":0,"env_keys":"CUDA_PATH","environment_diagnostics_present":false,"stdout_ok":true,"stdout_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","stdout_nvcc_exists":true,"stderr_bytes":0}
```

Registry was still invalid after the trigger, proving the success came from the explicit job
environment rather than a hidden registry repair:

```json
{"user_cuda_path":"C:\\__synapse_issue1709_invalid_cuda_root__","user_path_exists":false,"machine_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","process_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3"}
```

## Edge 3 - Empty Durable CUDA Root

Before trigger, HKCU `CUDA_PATH` existed and was the empty string:

```json
{"user_cuda_path":"","user_value_length":0,"machine_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","process_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3"}
```

Trigger: real `mcp__synapse.shell` with `operation=start`, job
`issue1709-empty-durable-20260716T1535`, command `cargo --version`.

After separate file readback:

```json
{"job_id":"issue1709-empty-durable-20260716T1535","status":"spawn_failed","pid":null,"exit_code":null,"error_code":"ACTION_TARGET_INVALID","diagnostic_code":"SYNAPSE_SHELL_ENV_DURABLE_MISSING","process_value":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","durable_value":"","effective_value":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","validation_path":"bin\\nvcc.exe","stdout_bytes":0,"stderr_bytes":0}
```

HKCU `CUDA_PATH` was then restored to its original absent state:

```json
{"user_value_exists":false,"user_cuda_path":null,"machine_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","machine_nvcc_exists":true}
```

## Edge 4 - Long-Lived Daemon Stale After Durable Registry Change

A manual daemon was launched and verified on PID `65160`. While it stayed alive, durable
`HKCU\Environment` `NVCC_APPEND_FLAGS` was changed from the old process value to a synthetic value:

```json
[
  {"case":"edge_stale_registry_mutation_before","daemon_pid":65160,"variable":"NVCC_APPEND_FLAGS","user":"-Xcompiler=/Zc:preprocessor","machine":null,"process_current_shell":"-Xcompiler=/Zc:preprocessor"},
  {"case":"edge_stale_registry_mutation_after_set_before_trigger","daemon_pid":65160,"variable":"NVCC_APPEND_FLAGS","user":"--synapse-issue1709-durable-readback-flag","machine":null,"process_current_shell":"-Xcompiler=/Zc:preprocessor","expected_child_value":"--synapse-issue1709-durable-readback-flag"}
]
```

Trigger: real `mcp__synapse.shell` with `operation=start`, job
`issue1709-stale-registry-nvcc-20260716T1555`, command `powershell.exe`, checking
`$env:NVCC_APPEND_FLAGS`.

After separate file readback:

```json
{"job_id":"issue1709-stale-registry-nvcc-20260716T1555","status":"ok","exit_code":0,"env_diag_count":1,"diagnostic_code":"SYNAPSE_SHELL_ENV_DAEMON_STALE","diagnostic_severity":"warning","diagnostic_process_value":"-Xcompiler=/Zc:preprocessor","diagnostic_durable_value":"--synapse-issue1709-durable-readback-flag","diagnostic_effective_value":"--synapse-issue1709-durable-readback-flag","diagnostic_source_of_truth":"HKCU\\Environment value NVCC_APPEND_FLAGS","stdout_ok":true,"stdout_nvcc_append_flags":"--synapse-issue1709-durable-readback-flag","stderr_bytes":0}
```

The durable value was restored immediately after:

```json
{"user":"-Xcompiler=/Zc:preprocessor","machine":null,"current_process":"-Xcompiler=/Zc:preprocessor"}
```

## Final Host Hygiene

The manual daemon was stopped by exact verified PID only. The normal scheduled daemon was restarted.

Final readback:

```json
{"daemon_pid":43372,"daemon_name":"synapse-mcp.exe","listener":{"LocalAddress":"127.0.0.1","LocalPort":7700,"State":2,"OwningProcess":43372},"task_state":"Running","user_cuda_path":null,"machine_cuda_path":"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.3","user_nvcc_append_flags":"-Xcompiler=/Zc:preprocessor","machine_nvcc_append_flags":null,"environment_broadcast_return":true,"environment_broadcast_last_error":0}
```

Final MCP health on PID `43372` returned `ok=true`, `tool_count=40`, and the same tool surface hash
`f457205fa2fb76db06c1157808fd54f417ace660f3398df74f80bb3c21383ffe`.

## Structural Checks

These are compile/lint checks only, not FSV.

- `cargo fmt --all`: passed during implementation.
- `cargo check -p synapse-mcp`: passed during implementation.
- Final `cargo fmt --all --check` and `cargo clippy --workspace --all-targets` were run before
  commit.
- No automated tests, benchmarks, FSV scripts, or FSV harnesses were added or run.

## Result

Issue #1709 acceptance is satisfied on the real configured host through the real
`mcp__synapse.shell` facade. Shell children no longer inherit stale daemon CUDA build variables for
the allowlisted build keys; durable Windows environment values are the child Source of Truth unless
the job explicitly supplies an override. CUDA-relevant commands fail closed before spawn when
durable `CUDA_PATH` is missing, empty, or invalid, and durable job artifacts contain structured
diagnostics showing the exact failing SoT and remediation.
