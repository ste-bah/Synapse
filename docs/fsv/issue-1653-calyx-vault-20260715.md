# Issue 1653 Calyx Vault FSV - 2026-07-15

Issue: https://github.com/ChrisRoyse/Synapse/issues/1653

This transcript records the manual full-state verification run for the daemon
owned Calyx/AsterVault lifecycle. The production acceptance trigger used the
installed Synapse HTTP MCP daemon on `127.0.0.1:7700` and the wired
`mcp__synapse` client. Temporary edge-case daemons used the same installed
binary with isolated vault/database directories.

## Research Inputs

- Rust `std::fs::File` documentation: dropping a file ignores close errors, so
  durable paths must explicitly flush/sync and surface errors before release.
- Rust `std::fs::File::sync_all` documentation: file contents and metadata must
  be synchronized explicitly when durability matters.
- Tokio graceful shutdown documentation: shutdown has three phases, including a
  cleanup phase where files/databases are flushed before tasks terminate.
- `fs2` file lock documentation: exclusive locks are process-level advisory
  locks and must be released deliberately after the durable close readback.
- Local Calyx `calyx-aster` fsync implementation: Windows directory fsync
  requires opening the directory with `FILE_FLAG_BACKUP_SEMANTICS`.

## Root Cause

Synapse had no daemon-owned Calyx vault lifecycle. The daemon could not prove a
single live owner for the vault, could not expose vault health in the MCP health
surface, and had no fail-closed startup/shutdown transaction around durable
open, flush, PID sidecar, lock release, and readback.

During installation validation, the setup candidate preflight also reused the
production default vault path while the production daemon was still live. Once
the vault became default-on, that made a healthy candidate look like a broken
build because it correctly failed on the production vault lock.

Review found one more durability gap before shipping: atomic identity/salt/PID
writes synced the staged file and rename target, but not the parent directory
entry after rename. The fix now syncs the parent directory and errors with a
typed remediation if that physical operation fails.

## Fix

- Added `crates/synapse-calyx` to own `AsterVault` open/close, stable
  `vault-identity.json`, machine salt, exclusive `vault.lock`, durable
  `vault.pid`, parent-directory fsync, typed errors, structured logging, and
  close readback.
- Integrated the vault into HTTP and stdio daemon startup/shutdown. Lifetime
  locks are released only after the Calyx close readback reports the PID
  sidecar removed and a re-lock probe succeeds.
- Added `calyx_vault` status to `health`, including vault paths, open phase,
  vault id, WAL sequence readback, and remediation fields.
- Added setup support for isolated candidate vault paths so install preflight
  does not contend with the production vault.

## Source Of Truth

- Production daemon process: `Get-CimInstance Win32_Process` for
  `synapse-mcp.exe`.
- Production listener: `Get-NetTCPConnection -LocalAddress 127.0.0.1
  -LocalPort 7700 -State Listen`.
- Wired MCP client: `mcp__synapse.health(detail=compact)` plus Codex tool
  discovery of the `mcp__synapse` facade surface.
- Production vault files:
  - `C:\Users\hotra\AppData\Roaming\synapse\vault\vault-identity.json`
  - `C:\Users\hotra\AppData\Roaming\synapse\vault\vault.pid`
  - `C:\Users\hotra\AppData\Roaming\synapse\vault\vault.lock`
  - `C:\Users\hotra\AppData\Roaming\synapse\vault\wal`
  - `C:\Users\hotra\AppData\Roaming\synapse\machine-salt.bin`
- Production lifecycle ledger:
  - `C:\Users\hotra\AppData\Local\synapse\db-daemon\daemon-tool-events.jsonl`
  - `C:\Users\hotra\AppData\Local\synapse\db-daemon\daemon-exit.jsonl`
- Edge-case readbacks:
  - `C:\Users\hotra\AppData\Local\synapse\manual-fsv\issue-1653-final-empty-20260715T163601292`
  - `C:\Users\hotra\AppData\Local\synapse\manual-fsv\issue-1653-final-invalid-20260715T163629526`
  - `C:\Users\hotra\AppData\Local\synapse\manual-fsv\issue-1653-final-lock-20260715T163711220`

## Environment

- Installed binary:
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`
- Installed binary SHA256:
  `2512CB7FDAE36A54F815B916B92A434EE404532F7E6D0CAD1B5373E51CD5483C`
- Final production daemon PID: `46828`
- Final production bind: `127.0.0.1:7700`
- Command line:
  `"C:\Users\hotra\.cargo\bin\synapse-mcp.exe" --mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\db-daemon --profile-dir C:\Users\hotra\.cargo\bin\profiles --log-level info`
- Wired MCP health:
  - `ok=true`
  - `pid=46828`
  - `tool_count=40`
  - `tool_surface_sha256=919b48c2ea68d4d3dd524057dfe77be21dda775457e0aaf380cb39ea7ab01e79`
  - `subsystems.calyx_vault.status=ok`
  - `calyx_vault.open=true`
  - `calyx_vault.phase=open`
  - `calyx_vault.vault_id=01KXKS2B1T5FPQ5722SJKNGSXR`
  - `calyx_vault.latest_seq=0`
  - `calyx_vault.last_recovered_seq=0`
- Codex tool discovery loaded the wired `mcp__synapse` facade surface and
  returned callable tools including `health`, `escalation`, `model`, `routine`,
  `target`, `browser_tabs`, `episode`, `verification`, `task`, and `session`.
- Setup wrote a same-agent restart handoff because this already-running Codex
  process predates the latest startup snapshot:
  `C:\Users\hotra\AppData\Local\synapse\codex-restart-handoffs\codex-restart-handoff-27508-20260715T213452668Z.json`.
  The daemon-side validation and the live `mcp__synapse.health` call both
  succeeded after that handoff was written.

## Production Happy Path

Expected: installing and starting the repo-built daemon opens the production
vault, writes the stable identity and PID sidecar, exposes `calyx_vault=ok` in
health, and records a lifecycle `opened` event.

Before trigger:

- Prior production daemon PID: `57096`
- Prior `calyx_vault_lifecycle` closed event:
  - `pid_sidecar_present_after_close=false`
  - `re_lock_probe_succeeded=true`
  - `safe_to_unlock=true`
  - `reason=http_endpoint`

Trigger:

- Ran `scripts\synapse-setup.ps1 -SourceDir C:\code\Synapse -ForceRestart
  -ActiveIssue 1653`.

After trigger:

- Process table: PID `7644`, executable
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Listener: `127.0.0.1:7700` owned by PID `7644`.
- Vault identity:
  - `schema_version=1`
  - `vault_id=01KXKS2B1T5FPQ5722SJKNGSXR`
- `vault.pid` contained PID `7644`.
- Lifecycle ledger contained PID `7644` `calyx_vault_lifecycle` status
  `opened` with:
  - `open=true`
  - `phase=open`
  - `latest_seq=0`
  - `last_recovered_seq=0`
  - production identity, salt, lock, PID, and vault paths.

## Edge Case 1 - Empty Vault Bootstrap

Expected: an empty configured vault directory bootstraps exactly once, creates
identity/salt/PID/layout files durably, reports healthy over MCP, and removes
the PID sidecar on graceful shutdown.

Before trigger:

- Root:
  `C:\Users\hotra\AppData\Local\synapse\manual-fsv\issue-1653-final-empty-20260715T163601292`
- `vaultExists=false`
- No listener on `127.0.0.1:7771`.

Trigger:

- Started the installed daemon with:
  `--mode http --bind 127.0.0.1:7771 --db <root>\db --calyx-vault-dir <root>\vault`.
- Called authenticated `health`.
- Called authenticated `/shutdown`.

After open:

- Process PID: `29088`.
- Health: `ok=true`, `calyx_vault.open=true`.
- Identity: `vault_id=01KXKVAJSVZDD0PQK6P4MDNJ92`.
- `vault.pid` contained PID `29088`.
- Machine salt SHA256:
  `C2799444E1B1B4FFCAF82BCA3E2A444409F810AC8D3FB699DF8D23257A74D58F`.
- Vault layout: `cf`, `locks`, `wal`, `vault-identity.json`, `vault.lock`,
  `vault.pid`.

After shutdown:

- Process alive: `false`.
- Listener on `7771`: absent.
- `vault.pid` exists: `false`.
- Shutdown HTTP status: `202`.
- Lifecycle ledger contained `opened` then `closed`.
- Closed readback:
  - `pid_sidecar_present_after_close=false`
  - `re_lock_probe_succeeded=true`
  - `safe_to_unlock=true`

## Edge Case 2 - Structurally Invalid Identity

Expected: a corrupt identity file fails startup closed, leaves the corrupt file
unchanged for operator recovery, removes any partial PID sidecar, releases the
lock, and logs the exact remediation.

Before trigger:

- Root:
  `C:\Users\hotra\AppData\Local\synapse\manual-fsv\issue-1653-final-invalid-20260715T163629526`
- Identity content:
  `{ "schema_version": 1, "vault_id": `
- Identity SHA256:
  `EC82F4FAED4D7A710575A5856E9E910314AE571FDC25A8C39DFEDD99FDE68A58`
- `vault.pid` absent.
- No listener on `127.0.0.1:7772`.

Trigger:

- Started the installed daemon with:
  `--mode http --bind 127.0.0.1:7772 --db <root>\db --calyx-vault-dir <root>\vault`.

After trigger:

- Process exit code: `1`.
- Listener on `7772`: absent.
- Identity content/hash unchanged:
  `EC82F4FAED4D7A710575A5856E9E910314AE571FDC25A8C39DFEDD99FDE68A58`.
- `vault.pid` exists: `false`.
- `vault.lock` exists: `true`.
- Logs contained:
  - `SYNAPSE_CALYX_OPEN_FAILURE_LOCK_CLEANED`
  - `primary_code=SYNAPSE_CALYX_IDENTITY_INVALID`
  - `pid_sidecar_present_after_close=false`
  - `re_lock_probe_succeeded=true`
  - `safe_to_unlock=true`
  - `STORAGE_OR_CALYX_OPEN_OR_MAINTENANCE_START_FAILED`
  - Remediation:
    `restore the vault identity files from backup or inspect the exact file named in the error`

## Edge Case 3 - Vault Lock Contention

Expected: a second daemon configured to the same vault fails closed before
serving, reports the holder PID, does not mutate the identity, and does not
disturb the first owner.

Before trigger:

- Root:
  `C:\Users\hotra\AppData\Local\synapse\manual-fsv\issue-1653-final-lock-20260715T163711220`
- No vault directory.
- No listeners on `127.0.0.1:7773` or `127.0.0.1:7774`.

Trigger:

- Started first daemon on `127.0.0.1:7773`.
- Started second daemon on `127.0.0.1:7774` with the same vault path.
- Shut down the first daemon through authenticated `/shutdown`.

After first open:

- First owner PID: `69612`.
- Health: `ok=true`, `calyx_vault.open=true`.
- Identity SHA256:
  `CEB133C6221FDB4C774E5C12C0B824B3575C5EF10921E1C815EBC1CD5745AD38`.
- `vault.pid` contained PID `69612`.

After second trigger:

- Second process exit code: `1`.
- Listener on `7774`: absent.
- First process alive: `true`.
- First health still `ok=true`, `calyx_vault.open=true`.
- Identity hash unchanged:
  `CEB133C6221FDB4C774E5C12C0B824B3575C5EF10921E1C815EBC1CD5745AD38`.
- `vault.pid` still contained PID `69612`.
- Logs contained:
  - `STORAGE_LOCK_CONTENDED`
  - `SYNAPSE_CALYX_LOCK_HELD`
  - `holder_readback.pid=69612`

After first shutdown:

- First process alive: `false`.
- Listener on `7773`: absent.
- `vault.pid` exists: `false`.
- Lifecycle ledger contained `opened` then `closed`.
- Closed readback:
  - `pid_sidecar_present_after_close=false`
  - `re_lock_probe_succeeded=true`
  - `safe_to_unlock=true`

## Edge Case 4 - Abrupt Production Death And Reopen

Expected: an abrupt production daemon death leaves a stale PID sidecar, the next
repo-built daemon detects the previous unclean run, preserves the stable vault
identity, opens the vault, rewrites the PID sidecar to the new PID, and exposes
healthy MCP state.

Before trigger:

- Process table: PID `7644`.
- Listener: `127.0.0.1:7700` owned by PID `7644`.
- Identity:
  - `schema_version=1`
  - `vault_id=01KXKS2B1T5FPQ5722SJKNGSXR`
- `vault.pid` contained PID `7644`.

Trigger:

- Killed exact verified PID `7644` with `Stop-Process -Id 7644 -Force`.
- Restarted the production scheduled task `SynapseMcpDaemon`.

After forced kill, before restart:

- PID `7644` process alive: `false`.
- Listener on `7700`: absent.
- `vault.pid` still contained PID `7644`.
- Identity unchanged:
  `vault_id=01KXKS2B1T5FPQ5722SJKNGSXR`.

After restart:

- Process table: PID `46828`.
- Listener: `127.0.0.1:7700` owned by PID `46828`.
- Only live `synapse-mcp.exe` process: PID `46828`.
- `vault.pid` contained PID `46828`.
- Identity unchanged:
  `vault_id=01KXKS2B1T5FPQ5722SJKNGSXR`.
- Machine salt SHA256:
  `6652F74BA4A792D986820E69825EEBE14A70E068CD42B45BC2BF3F0650E6C66D`.
- Vault layout: `cf`, `locks`, `wal`, `vault-identity.json`,
  `vault.lock`, `vault.pid`.
- Wired MCP health:
  - `ok=true`
  - `pid=46828`
  - `tool_count=40`
  - `calyx_vault.status=ok`
  - `calyx_vault.open=true`
  - `calyx_vault.vault_id=01KXKS2B1T5FPQ5722SJKNGSXR`
- Lifecycle ledger contained PID `46828` `calyx_vault_lifecycle` status
  `opened`.
- Exit ledger contained PID `7644` `previous_run_unclean` with:
  - `cause=process_missing_on_startup`
  - `detail.new_pid=46828`
  - `reason=daemon-run-current had no ended_at_unix_ms when this daemon acquired the DB lock`
