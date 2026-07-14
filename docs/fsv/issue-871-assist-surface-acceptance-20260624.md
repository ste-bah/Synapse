# Issue #871 assist-surface acceptance FSV

Date: 2026-06-24
Host: Windows user `hotra`
Daemon bind: `127.0.0.1:7700`
Branch: `main`

> **Current D1 classification (2026-07-13):** The renamed script referenced
> below is supporting diagnostic automation only; it does not perform or accept
> FSV. Its output was not, and is not now, sufficient by itself for acceptance.
> The historical transcript values remain evidence from the separately observed
> manual run. Current acceptance requires an agent to use the strict production
> MCP client and independently read each physical Source of Truth before and
> after every manual trigger.

## Setup and live state

- Source was current with `origin/main` before the run: `7cd9a15af6a59c4096acf6b713fc56e69202eb74`.
- Worktrees: single worktree at `C:/code/synapse`; no extra local branches beyond `main`.
- `scripts/synapse-setup.ps1 -SourceDir C:\code\synapse -ForceRestart` rebuilt from local source, installed `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, restarted scheduled task `SynapseMcpDaemon`, rewired Codex/Claude HTTP MCP entries, and verified the Chrome bridge.
- Final installed daemon PID: `8588`.
- Same Codex terminal reconnected through `mcp__synapse.health` after restart; no new Codex terminal was required.
- Chrome bridge: `ok`, extension `leoocgnkjnplbfdbklajepahofecgfbk`, active profile `Profile 5`, build `synapse-chrome-bridge-2026-06-24-mousedown-click-v3`.
- The installer reported Chrome bridge auto-install/readback: `auto_install_attempted=True`, active profile installed `True`.
- Windows notifications were initially disabled for this user (`NotificationSetting=2`). For the FSV, the user notification state was repaired by setting `HKCU\Software\Microsoft\Windows\CurrentVersion\PushNotifications\ToastEnabled=1`, `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\NOC_GLOBAL_SETTING_TOASTS_ENABLED=1`, `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\Synapse.Daemon\Enabled=1`, then restarting `WpnUserService_100839`.

## Code and tool changes verified

- Historical note: the former #871 diagnostic automation was retired by #1644. During
  this archived run it:
  - opens two independent HTTP MCP sessions,
  - uses only an already-open Chrome window,
  - verifies physical Action Center toast XML and accept URI,
  - checks MCP/dashboard/tray approval Source of Truth hashes,
  - compares dashboard panels to MCP/HTTP storage, timeline, and daemon state,
  - toggles the tray recorder control through `synapse-overlay --toggle-once`,
  - blocks a live `approval_gate`, accepts it from the dashboard endpoint, and verifies the gate returns `allow`.
- Historical note: the retired `synapse-fsv-toast-history` helper was used during
  this 2026-06-24 acceptance run to read/remove `Synapse.Daemon` Action Center
  history rows and extract approval action URIs from physical toast XML. It is no
  longer a package binary or supported diagnostic entry point; current
  verification must use the production Synapse surfaces and manual SoT readback
  required by D1.
- Added `synapse-overlay --toggle-once` so the real tray pause/resume control path is testable without synthetic Win32 tray clicks.
- Hardened `approval_protocol` parsing for Windows ShellExecute handoff variants (`"synapse-approval://..."` and `synapse-approval://decide/?...`).
- Changed dashboard storage summary to use exact row counts (`storage_cf_row_counts`) so dashboard storage SoT matches `storage_inspect`.

## Discrepancies found and fixed

1. Dashboard storage panel used RocksDB estimated row counts while `storage_inspect` returned exact counts. The FSV found `CF_KV` mismatch. Fixed by changing `inspect_storage_summary()` to keep size estimates but use exact row counts and label `metrics_mode` as `rocksdb_live_data_size_estimates_exact_row_counts`.
2. Physical toast accept URI worked when the installed handler was invoked directly, but Windows ShellExecute returned exit `1` and left the row pending. Fixed by accepting quoted and `decide/` protocol URI variants in `approval_protocol::ProtocolActivationRequest::parse`; focused tests now cover these variants.

## Manual FSV transcript

Historical command: this run used the now-retired #871 diagnostic script. The
script path is intentionally omitted so this archive does not provide a current
executable verification instruction.

Result:

```json
{
  "issue": 871,
  "marker": "issue871-fsv-20260624-161523",
  "daemon": {
    "pid": 8588,
    "bind": "127.0.0.1:7700",
    "daemon_tool_count": 216,
    "daemon_tool_surface_sha256": "37cc01612ca64f48d975db2850e08511ec9fc23192d37fa4021157bf14767b4c",
    "visible_tool_count": 173,
    "visible_tool_surface_sha256": "bbadb49de3fff07adc5aa9d7f88b0c0428769b892aa665d4611d6997419d7375",
    "chrome_bridge_status": "ok"
  },
  "concurrent_sessions": {
    "session_a": "a3de407a-7b20-4581-bf31-d41d7ab46476",
    "session_b": "273bf362-5c60-4b2b-a38c-28171333c556",
    "live_session_count": 3,
    "dashboard_total_count": 8
  },
  "browser_dashboard": {
    "existing_chrome_hwnd": 524970,
    "cdp_target_id": "chrome-tab:589708919",
    "url": "http://127.0.0.1:7700/dashboard?fsv=issue871-fsv-20260624-161523#/system",
    "dom_title": "Synapse Command Center",
    "screenshot_path": "C:\\Users\\hotra\\AppData\\Local\\Temp\\issue871-fsv-20260624-161523-dashboard.png",
    "screenshot_bytes": 0,
    "screenshot_error": "browser_screenshot Chrome bridge capture failed: image readback failed"
  },
  "E1_toast_accept": {
    "approval_id": "apr1-019efb7d42ba700090451ff042059fcd",
    "toast_tag": "dk-8eb88452d837d7824e4252029f9c9c3f",
    "activation_id": "actv1-019efb7d42ec76319b9701f37d6d9787",
    "toast_xml_sha256": "sha256:7c318716e7a4b6092415ebbcacda86a50247d48fb567c0e186c64708cdad241d",
    "accept_uri_redacted": "synapse-approval://decide?bind=127.0.0.1%3A7700&approval_id=apr1-019efb7d42ba700090451ff042059fcd&activation_id=actv1-019efb7d42ec76319b9701f37d6d9787&token=<redacted>&decision=accept",
    "after_status": "accepted",
    "decided_by_session": "approval_protocol",
    "item_row_sha256": "sha256:1535a5a937ac27970e432be34ad7469cf7cbf9309727094f04a0645f3c5925d9"
  },
  "E2_queue_sot": {
    "dashboard_pending_row_hash": "sha256:613beb25690d4579f8071e570dbc5a82adde90fb6b4025846bf3574d741bb8c9",
    "tray_pending_row_hash": "sha256:613beb25690d4579f8071e570dbc5a82adde90fb6b4025846bf3574d741bb8c9",
    "mcp_pending_row_hash": "sha256:613beb25690d4579f8071e570dbc5a82adde90fb6b4025846bf3574d741bb8c9"
  },
  "E3_dashboard_sot": {
    "daemon_pid_match": true,
    "timeline_total_rows": 4712,
    "storage_cf_kv_rows": 1655,
    "panel_statuses": "dashboard_assets=ok;auth=ok;daemon=ok;sessions=ok;lease=ok;storage=ok;target_claims=ok;timeline=ok;demo_recording=ok;events=ok;hidden_desktops=ok;cdp_attachments=ok;shell_jobs=ok;command_audit=ok;tasks=ok;approvals=ok;suggestions=ok;armed_runs=ok;agent_transcripts=ok;agent_cost=ok;agent_stats=ok;context=ok;hygiene=ok;local_models=ok"
  },
  "E4_tray_control": {
    "status_once_pending_approvals": 2,
    "initial_recorder_paused": false,
    "toggled_recorder_paused": true,
    "restored_recorder_paused": false
  },
  "E5_controls_control": {
    "gate_approval_id": "apr1-019efb7d79257b0180cd064635b25d14",
    "dashboard_decision_status": "accepted",
    "gate_verdict": "allow",
    "updated_command": "git push origin issue871-fsv-20260624-161523"
  },
  "discrepancies": []
}
```

Note: `browser_screenshot` failed in the Chrome bridge image readback path, but this was non-blocking for #871. The FSV still verified the existing Chrome tab, dashboard URL, title, and DOM content via `browser_content` / `browser_evaluate`.

## Verification commands

- `cargo test -p synapse-mcp --bin synapse-mcp approval_protocol -- --nocapture`
- `cargo build -p synapse-overlay`
- Historical PowerShell parser check for the now-retired #871 diagnostic script.

The closed #871 diagnostic script referenced by this archived document was
retired by #1644 and is no longer a current verification command.

