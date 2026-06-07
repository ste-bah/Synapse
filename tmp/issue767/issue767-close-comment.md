Fixed in `13077fc` (`fix(mcp): allow declared act_press foreground transitions [skip ci]`).

Root cause:
- `act_press verify_delta` treated every foreground identity change as `ACTION_FOREGROUND_LOST`.
- That was correct for unexpected focus loss, but wrong for intentional key-triggered transitions such as a launcher opening/focusing another app.

Implementation:
- Added explicit `act_press` policy fields: `allow_foreground_change`, `expected_foreground_process_regex`, and `expected_foreground_title_regex`.
- Default remains fail-closed: foreground changes still return `ACTION_FOREGROUND_LOST` unless explicitly allowed.
- Expected regexes are validated before input is sent. Invalid policy returns `TOOL_PARAMS_INVALID` and does not press the key.
- Mismatches return structured `ACTION_FOREGROUND_LOST` with `reason=foreground_change_policy_mismatch`, policy readback, match booleans, and before/after signatures.
- Tool schemas now describe the policy so strict clients can discover it.

Research checked:
- Microsoft `GetForegroundWindow`: foreground is the window the user is working with.
- Microsoft `SetForegroundWindow`: foreground activation is constrained and focus changes are meaningful OS state.
- Microsoft UI Automation Events overview: UI changes/focus changes need separate observation, not a return-value-only verdict.

Supporting checks run locally, no GitHub Actions/CI:
- `cargo fmt --all --check` -> passed.
- `cargo test -p synapse-mcp --bin synapse-mcp act_press_ -- --nocapture` -> 6 passed.
- `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture` -> passed.
- `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture` -> passed.
- `cargo check -p synapse-mcp --bin synapse-mcp` -> passed.

Manual Source-of-Truth verification:
- MCP daemon SoT: `synapse-mcp.exe` PID `20720`, HTTP bind `127.0.0.1:7700`, `health.ok=true`, input lease free.
- Trigger surface: real `mcp__synapse.act_press` calls. For new schema fields, a fresh primary `codex exec` process loaded the real MCP server and its JSONL log contains `mcp_tool_call` rows for `synapse/health` and `synapse/act_press`.
- Storage SoT: `CF_ACTION_LOG` plus redacted audit export at `C:\code\Synapse\tmp\issue767\audit-export-terminal`.
- UI SoT: separate `observe` reads of foreground/window HWNDs after the triggers.

Manual cases:
- Happy path: `SYN767_HAPPY2` PowerShell launcher waited at `Read-Host`; fresh primary Codex called `act_press` with `allow_foreground_change=true`, expected process `(?i)^notepad\\.exe$`, expected title `(?i)notepad`. `CF_ACTION_LOG` rows `seq=13/14` show `status=ok`; separate `observe(window_hwnd=20778646)` read `Notepad.exe`, title `Untitled - Notepad`.
- Edge 1 invalid regex: `SYN767_EDGE_INVALID` stayed foreground; fresh primary Codex called `expected_foreground_title_regex="["`. Final error was `TOOL_PARAMS_INVALID`, `reason=invalid_expected_foreground_regex`, field `expected_foreground_title_regex`; process list showed no Notepad child from launcher PID `48764`; `CF_ACTION_LOG` rows `seq=19/20` captured the error before input.
- Edge 2 default strict behavior: with no foreground-change policy, pressing Enter opened Notepad, but `verify_delta` returned `ACTION_FOREGROUND_LOST`, `reason=unexpected_foreground_change`; separate `observe(window_hwnd=14617440)` read `Notepad.exe`, title `Untitled - Notepad`; `CF_ACTION_LOG` rows `seq=22/23` captured before/after.
- Edge 3 wrong expected foreground: `SYN767_EDGE_WRONG` opened Notepad while policy expected Calculator. Fresh primary Codex final JSON showed `ACTION_FOREGROUND_LOST`, `reason=foreground_change_policy_mismatch`, `process_match=false`, `title_match=false`; separate `observe(window_hwnd=15797078)` read `Notepad.exe`, title `Untitled - Notepad`; `CF_ACTION_LOG` rows `seq=27/28` captured policy, match booleans, and before/after.

Audit export readback:
- `manifest.json` SHA-256 `8D9ECB01A21A9513A2BD943331AD3017EB98F8A36D226FB887B90B951C023249`.
- `rows.json` SHA-256 `65328D0AF5FAB21905CF4E8AC56802B754D52E23B0E8CCC1C3A117D2569170FF`.
- `redaction_report.json` SHA-256 `D515598C0DF106ACFE869C76E017E8ECED433BB3497BEDBAD66D16716931C839`.
- Exported rows contain the clean happy `observed expected foreground transition`, invalid regex `TOOL_PARAMS_INVALID`, default `unexpected_foreground_change`, and wrong-policy `foreground_change_policy_mismatch` outcomes.

Host hygiene after verification:
- Stopped only exact PIDs spawned for this issue and their exact child Notepad PIDs.
- No `SYN767` marker processes remained.
- No Cargo/rustc/link helper processes remained.
- Setup maintenance lock absent.
- Daemon PID `20720` remained healthy on `127.0.0.1:7700`; input lease remained free.

Closing #767.
