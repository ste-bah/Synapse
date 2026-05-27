# 00 — Vision and Scope

## 1. Mission

**Give AI agents a body.** Frontier models (Claude, GPT, Gemini class) reason well but are blind, deaf, limbless on a real computer. Synapse exposes the OS, every visible window, and every running game as a structured, queryable, controllable surface over MCP.

Agent thinks. Synapse perceives, acts, reflexes.

## 2. Problem statement

An AI agent operating Windows today has three bad choices:

1. **Screenshot loops** — capture PNG, pay 1500-2500 vision tokens, wait for model to find a button, send coord, repeat. Cost: ~$0.05-$0.20/step. Latency: 500ms-3000ms/action. No real-time reaction. Fails on games.
2. **Browser-only tools** — Playwright/Puppeteer give rich DOM but cover ~5% of the desktop. Useless for native apps and games.
3. **Custom OS automation libraries** (PyAutoGUI, AutoIt) — coordinate-based, brittle, no semantic state, no event push, no game support.

None meet the human-operator bar: **see structure, react instantly, act precisely, work everywhere**.

Synapse is the missing primitive.

## 3. Target users

| Persona | Need | How they use Synapse |
|---|---|---|
| **AI engineer building a desktop agent** | Reliable, fast, semantic perception + action across any app | Embed `synapse-mcp` as a tool in Claude Code / Codex / custom runner; build agent behaviors in prompts |
| **Game-AI researcher** | Low-latency observation + control loop for arbitrary games | Use Synapse as the I/O layer; bring their own perception model when defaults aren't enough |
| **Accessibility tooling builder** | Programmatic operation of inaccessible apps | Use UIA path for accessible apps, capture+OCR fallback for the rest |
| **QA / automation engineer** | Replace flaky pixel-matching with structured action | Use a11y path for stable element references; replay test logs from RocksDB |
| **Hobbyist letting Claude play their game** | Set it up once, let the agent loop on a game while they watch | Install via `cargo install`, point Claude Desktop at it, give a goal |
| **Speedrunning / TAS-adjacent research** | Frame-perfect inputs with structured state | Hardware HID gateway + reflex runtime |

**Not** target users:
- Mass-account operators running parallel unattended sessions. Single-machine system.
- Mobile or console operators. Desktop only.

## 4. Why now

Three changes in the last 18 months:

1. **MCP became a real standard.** Streamable HTTP transport (March 2025) supports fast tool calls and long-lived event streams on one endpoint. Every major agent client (Claude Desktop, Cursor, Codex, VS Code, ChatGPT Desktop) speaks it natively.
2. **Frontier models gained tool-use loops.** Claude 3.5+, GPT-4o+, Gemini 2+ plan and reflect across hundreds of tool calls. The agent is the outer loop — we feed it observations and execute its commands.
3. **Consumer GPUs got capable.** RTX 4090 / 5090 runs YOLO-class detectors at 200+ FPS, ConvNeXt-tiny at 500+ FPS, small VLMs at 5-10 FPS. Real-time game perception on a single workstation is trivially fast.

## 5. Concrete capabilities at v1

When the agent connects to `synapse-mcp`, it gains these capabilities for any foreground Windows app or game:

### See (perception)

- Read structured tree of every accessible window (UIA): names, roles, AutomationIds, bounding boxes, patterns, focus, enabled state
- Read DOM + accessibility tree of any Chromium-based browser (CDP)
- Read structured state of apps with known automation APIs (Office COM, terminal PTY, VS Code LSP via extension, Slack/Discord APIs)
- Capture window or full desktop as GPU texture at 60+ fps with zero CPU copy
- Run small object-detection model (YOLO-class, ~20MB ONNX) on frames at 50+ fps
- OCR screen/subregion via WinRT `Windows.Media.Ocr` (no Tesseract) or fine-tuned CRNN
- Capture + transcribe system audio (WASAPI loopback + small STT)
- Detect spatial direction of stereo audio (FPS footstep direction)
- Watch filesystem (`ReadDirectoryChangesW`)
- Watch processes, sockets, clipboard
- Subscribe to all above as push event stream over MCP notifications

### Act (action)

- Click, double-click, right-click, drag, scroll (mouse — software, virtual driver, or hardware HID)
- Type text, press keys, hold modifiers, send chord combos (keyboard — same three paths)
- Drive virtual Xbox 360 / DualShock 4 controller (ViGEm)
- Send analog stick deltas, trigger pressures, button presses
- Move cursor with smooth aim curves (Bezier with micro-tremor + variable timing)
- Type with configurable keystroke pacing (Gaussian inter-arrival)
- Send frame-perfect input sequences (fighting-game motion inputs)
- Read/write clipboard
- Launch / focus / close windows
- Run shell commands (gated, optional)

### React (reflex runtime)

- Continuous aim-tracking: "lock onto target X until told to stop"
- Hold patterns: WASD strafe, bunny-hop, sprint-jump-crouch chains
- Frame-accurate combo sequencers
- `on_event` reactive bindings: "when low_hp event fires, press the medkit slot"
- Auto-dismiss popups by class+text matching
- Watchdog timers: "if no progress in 5s, abort and notify model"

### Persist + observe

- All events recorded to RocksDB with timestamps for replay debug
- All MCP requests/responses traced with `tracing` + OTLP export
- Per-app/per-game profiles (HUD layout, keymap, capture region) loaded on-demand
- Replay tool to play back any session deterministically

## 6. Non-goals (explicitly NOT built)

Out of scope. NEVER accept feature requests for these without an ADR.

1. **No goal planning, no MCTS, no GOAP, no skill libraries.** Agent is the planner. We do not invent per-game skill ontologies, do not maintain skill graphs, do not run plan-space search. Multi-step composition lives in agent tokens, not ours.
2. **No inner LLM.** Synapse loads no large model. Optional vision models stay small (≤100M params). The connecting agent is the only "intelligence."
3. **No prediction / world model / learning head.** No future-state prediction, no reward signal, no runtime weight adaptation. Optional model inference is perception (detection / OCR), not prediction or RL.
4. **No unsupported process-manipulation or device-identity features.** Hardware HID supports accessibility, QA, research, simulation, and local game-control workflows; bundled firmware identifies itself plainly.
5. **No general-purpose RPA / web scraping framework.** Browser CDP ships because games and apps have web subviews; not a Playwright competitor.
6. **No mobile, console, embedded.** Windows desktop only at v1. Linux/macOS v2.
7. **No multiplayer / multi-machine orchestration.** One agent, one machine, one Synapse server.
8. **No cloud service.** Runs entirely on operator's machine. No telemetry leaves the box without explicit opt-in.

## 7. Success criteria

Synapse v1 is successful when:

1. **An agent driving Claude via stdio MCP can open Notepad, type a paragraph, save to a path, verify file exists — ≤8 tool calls, ≤2500 tokens.** Today: ~30+ screenshot-based steps, ~30K tokens.
2. **Agent completes a 30-minute single-player game session** (e.g., Minecraft from spawn → build shelter → kill a mob) using only Synapse, no human intervention, ≤200 tool calls.
3. **Agent reacts to in-game events at frame rate** for at least one supported FPS — "track that enemy and shoot if visible," reflex runtime delivers clicked shot within 33ms of enemy becoming visible.
4. **Steady-state token cost ≤ 800 tokens/turn** for structured observation, vs. ~1800 for a screenshot of the same scene.
5. **Detection inference + capture stays under 16ms p99** on a 5090.
6. **No silent failures.** Every MCP tool failing to do its work returns a structured error code, never `success: true` shell.
7. **One-command install** on fresh Windows 11: `winget install Nefarius.ViGEmBus; cargo install --git ... synapse-mcp`.

## 8. Anti-success criteria (failure modes to avoid)

| Failure mode | How we avoid |
|---|---|
| Screenshot-loop fallback masquerading as "structured" perception | Hard rule: `observe()` returns structured data; if both a11y and detection fail, return `OBSERVE_NO_PERCEPTION_AVAILABLE` error with diagnostics, never silently include a screenshot |
| Slow path becoming the only path | Per-tool p99 latency budgets enforced through local benchmark exports and manual review; perf regressions block merge |
| Sensitive input paths enabled by accident | Hardware HID, shell, process launch, non-loopback networking, and redaction changes require explicit operator configuration |
| Tool-bloat (200+ MCP tools, agent confused) | Hard cap: current approved live surface is 34 tools after M4/M5 expansion. Anything else is a profile, a parameter, or a sub-command of an existing tool unless an ADR approves the cap change |
| Token bloat per observation | Hard cap: `observe()` returns ≤ 1500 tokens by default; agent must `expand(slot)` for more |
| Per-game special-casing in core code | Per-game logic lives in declarative profiles (`profiles/<id>.toml`), not Rust code |
| Build complexity sprawl | Workspace ≤ 15 crates; one binary; no procmacro forests; no build.rs that hits the network |

## 9. Definition of "done" for the PRD

1. All 18 docs in this directory exist, internally consistent.
2. Architecture in `01` matches structs in `06`, tools in `05`, milestones in `15`.
3. Every external dep named with specific crate version range.
4. Every external service / OS API identified by exact name + minimum Windows version.
5. A new reader can sit down, read the PRD, begin coding `synapse-core` without a clarifying question.

## 10. v1 vs deferred

| At v1 | Deferred |
|---|---|
| Windows 11 / Windows 10 21H2+ | Linux Wayland/X11, macOS |
| stdio + Streamable HTTP MCP transports | WebSocket, IPC pipes |
| UIA, CDP (Chromium), file watch, clipboard, processes | AT-SPI (Linux), AX (macOS), Wayland-specific |
| Software input, ViGEm virtual pad, RP2040 HID | Interception driver, kernel-level hooks |
| YOLO + ConvNeXt detection, WinRT OCR | Custom segmentation, depth-from-stereo, large VLM inference |
| WASAPI loopback + simple direction estimate | HRTF-accurate spatial audio, room acoustics |
| Per-app / per-game profile TOML | Profile auto-generation from one-shot bootstrap session |
| 5+ shipped profiles (Notepad, VS Code, Chrome, Minecraft, one FPS) | Marketplace of community profiles |
| Replay log, debug overlay | Visual session reviewer, timeline scrubber UI |
| MIT/Apache-2.0 dual license | Commercial OEM license tier |

## 11. Risk register (top items)

| Risk | Impact | Mitigation |
|---|---|---|
| Microsoft tightens GPU capture permissioning in a future Windows update | High — perception breaks | Maintain DXGI Output Duplication fallback in addition to Graphics Capture API |
| A game ignores or mishandles virtual controller input | Medium — some game support narrows | Hardware HID remains an optional physical-input path for accessibility, rigs, and local game profiles |
| MCP transport spec changes again | Low — minor refactor | Stay on official `rmcp` crate; track spec releases |
| Vision-model dependency on bundled ONNX files | Medium — install size, licensing | Default-bundle only models with permissive licenses; download larger models on first run with explicit consent |
| RocksDB on Windows is sometimes finicky | Low | Pin a known-good `rocksdb` crate version; M3 uses RocksDB only per ADR-0002 |
| Hardware HID requires user to solder/buy a $4 board | Low | Make the gateway optional; document the use case clearly; ship pre-built firmware images |
| Claude/Codex/Cursor change MCP client behavior | Low | Local compatibility checks against each client; don't depend on undocumented behavior |

## 12. The single line that decides everything

When in doubt: **the model is the brain, Synapse is the body. If a feature requires the body to make strategic decisions, it doesn't belong here. If a feature requires the body to react faster than the model possibly can, it absolutely belongs here.**
