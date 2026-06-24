# Issue 876 Hygiene Acceptance FSV - 2026-06-24

Issue: https://github.com/ChrisRoyse/Synapse/issues/876

This transcript records the manual full-state verification run for the hygiene
acceptance issue. The run used the installed Synapse HTTP MCP daemon on
`127.0.0.1:7700`, a live Chrome bridge, and physical Synapse storage.

## Environment

- Repo: `C:\code\synapse`
- Branch: `main`
- Daemon PID: `32912`
- Installed binary SHA256: `23819CF7D07024DF6628BF1A479F3F3F2197B0B732279D47C659AFF0754A58B8`
- Direct `/health` implementation tool count: `216`
- Direct `/health` implementation tool surface SHA256: `37cc01612ca64f48d975db2850e08511ec9fc23192d37fa4021157bf14767b4c`
- Normal Codex profile visible tool count: `166`
- Normal Codex profile hash: `sha256:0fb20e1cc24fe51ff28bab1b8dc36bef22fb4b164731d4f965b9c1257e87cd53`
- Chrome bridge status: `ok`
- Bridge build: `synapse-chrome-bridge-2026-06-24-mousedown-click-v3`
- Installed Chrome profile: `Profile 5`
- FSV MCP session: `67d1b5d0-f374-4b22-a2b4-825d84d1aec8`
- Unique marker: `issue876-fsv-20260624-131417`

The direct `/health` value reports the implementation surface before profile
filtering. The same-process normal Codex profile readback reported 166 visible
tools, including the hygiene, episode, routine, and timeline cleanup tools used
for this FSV.

The current Codex process had stale lazy-loaded tool schemas after the daemon
reload, so the FSV called the authenticated Streamable HTTP MCP endpoint
directly from the existing process. No new Codex terminal was opened or needed.

## Code Surface

The normal profile now exposes the tools required to verify and clean the
hygiene pipeline end to end:

- `hygiene_report`
- `timeline_redact`
- `episode_get`
- `episode_list`
- `episode_segment`
- `routine_mine`
- `routine_list`
- `routine_inspect`

`timeline_redact` is classified as destructive because it mutates timeline
storage and invalidates derived state.

## Live Browser Injection

The FSV created two target-owned localhost pages in the already-open Chrome
window, one web page and one document-style page. Both contained the planted
injection marker. The Chrome bridge inspected them without requiring a foreground
tab switch:

- Chrome window HWND: `524970`
- Chrome window title: `Synapse Command Center - Google Chrome`
- Localhost page server port: `52406`
- Web target: `chrome-tab:589708887`
- Document target: `chrome-tab:589708888`
- Web annotations: `1`
- Document annotations: `1`
- Web required foreground: `false`
- Document required foreground: `false`
- Web active after capture: `false`
- Document active after capture: `false`

`hygiene_scan_text` found the planted injection text once and did not flag the
benign control text:

- Injected matches: `1`
- Benign matches: `0`

## Storage And Derived State

The FSV seeded bounded probe rows into physical timeline storage, then exercised
the live storage hygiene, episode, routine, and report path:

- Pre-cleanup dry-run matched old marker rows: `0`
- Pre-cleanup deleted old marker rows: `0`
- Seeded timeline rows: `20`
- Poisoned rows: `5`
- `episode_segment` rows written: `15`
- `routine_mine` routines written: `1`
- Routine ID: `rt1-1db508a0392da884`
- `hygiene_scan_storage` flags written: `5`
- Seeded flags found by exact source key: `5`
- `hygiene_report` seed flags: `5`
- `hygiene_report` flags linked to routine: `5`

This proves the planted prompt-injection rows were visible in storage, flagged
by the hygiene scanner, and linked through derived episode/routine provenance.

## Cleanup And Invalidation

The cleanup phase redacted one flagged row, purged the remaining flagged rows,
and verified derived-state invalidation:

- Redacted flag count: `1`
- Purged flag count: `4`
- `timeline_redact` dry-run rows: `1`
- `timeline_redact` deleted rows: `1`
- Redaction audit key: `18bc1732a0b8450cffff0005`
- Taint records written: `3`
- Tainted routine IDs: `rt1-1db508a0392da884`
- Routine tainted: `true`
- `timeline_purge` dry-run matched rows: `4`
- `timeline_purge` deleted rows: `4`
- Purge audit key: `18bc1732c0abc370ffff0006`
- Post-injection timeline matches: `0`
- Final marker cleanup dry-run matched probe rows: `16`
- Final marker cleanup deleted probe rows: `16`
- Final marker probe timeline matches in the probe range: `0`
- Live scoped readback after the run: `timeline_search` for
  `issue876-fsv-20260624-131417` with `kinds=["browser_nav"]` returned `0`
  matches, while the retained `purge` audit row remained searchable.

The FSV session was explicitly ended at completion. `session_end` reported
`failure_count=0`, `marked_terminated=true`, and `reason=explicit_session_end`.

## False-Positive Sweep

The FSV ran a clean-day sweep against real data from `2026-06-19`:

- Rows scanned: `175`
- Rows flagged: `0`
- False-positive rate: `0.0`

## Acceptance Mapping

- Planted on-screen injection: PASS. A web page and a document-style page were
  loaded in the existing Chrome window and inspected through the live bridge.
- Realtime annotation: PASS. Both target pages produced one hygiene annotation.
- Batch storage flags: PASS. Five poisoned physical timeline rows produced five
  hygiene flags.
- Hygiene report provenance: PASS. `hygiene_report` linked all five seed flags
  to routine `rt1-1db508a0392da884`.
- Cleanup: PASS. One flagged row was redacted and four remaining flagged rows
  were purged from physical timeline storage.
- Invalidation: PASS. Cleanup wrote three taint records and marked the derived
  routine as tainted.
- Auditability: PASS. Redaction and purge audit keys were returned and retained.
- False-positive floor: PASS. A real clean-day sweep scanned 175 rows and
  produced zero hygiene flags.
- Reconnect guarantee: PASS. The final daemon was installed, reloaded, and
  reachable from the same Codex process via HTTP MCP with no new terminal.
