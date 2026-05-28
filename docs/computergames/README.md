# Synapse — Real-Time Computer-Use & Game-Control MCP for AI Agents

**Project codename:** Synapse ("the nerve connection between AI brain and computer body"; rename freely).
**Language:** Rust, edition 2024, using the current installed stable toolchain for M0 (see `docs/adr/0001-current-rust-and-dependencies.md`). All Rust end-to-end; no Python, no C/C++ glue beyond unavoidable FFI to Windows SDK / RocksDB / WASAPI / RP2040 SDK.
**Target host:** Windows 11 x64 (primary), Windows 10 x64 (best-effort). Linux/macOS deferred to v2.
**License:** MIT or Apache-2.0 dual.
**Repo:** fresh, independent. Clean-room, no upstream vendor dependencies.

**Agent doctrine:** repository agents must read `AGENTS.md`. Manual FSV on the
configured Windows host is the shipping gate. Automated tests, scripts,
benchmarks, GitHub Actions, and CI are supporting evidence only and must never
be presented as FSV.
Missing configured-host prerequisites are acquisition/setup work, not blockers:
agents must use Synapse/local control as the operator-equivalent host control
surface with full local computer-control responsibility, plus normal OS, shell,
browser, package-manager, and device-management workflows, to make missing
tools, drivers, models, files, services, account state, firmware, and hardware
surfaces real. They must then read the physical source of truth directly and
ask only before hard-to-reverse external actions.
Do not stop at "missing": Synapse gives the agent the same practical local
host-control ability the operator has at this computer. If the operator could
do it locally, the agent must use Synapse/local host control to make it happen,
then inspect the resulting source of truth.
Missing local state creates the next action for the agent and must be made
real, not handed back to the operator or treated as a blocker while reversible
host work remains.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.
Browser downloads, GUI installers, Device Manager checks, package-manager
installs, model/file generation, firmware flashing, app launching, USB/COM
inspection, and UI inspection are agent-owned work when they are reversible on
this host. Blocking is limited to the exact hard-to-reverse external action
left after that work is exhausted.

---

## What this is

Synapse is an **MCP (Model Context Protocol) server** giving any MCP-aware AI agent (Claude, Codex, Cursor, custom runners) a fast, structured, low-token interface to **see**, **hear**, **act on**, and **react inside** any Windows desktop application — across two modes:

| Mode | Examples | Primary perception path |
|---|---|---|
| **Computer-use** | VS Code, Excel, Outlook, Slack, browsers, file explorer, terminals, design tools | Accessibility tree (UIA), DOM (CDP), app-specific APIs, OS event hooks |
| **Game-control** | Single-player games, modded multiplayer, browser/Roblox games, real-time titles | GPU frame capture + small detection CNN, HUD OCR, spatial audio, game-specific RAM hooks where ethically allowed |

Both modes share the Rust workspace, MCP tool surface, action subsystem, and sub-frame reflex runtime. Perception auto-selects the cheapest path with sufficient fidelity. Rich a11y tree → use it (sub-millisecond, zero tokens on pixels). GPU-only render (most games, some Electron, canvas-heavy tools) → capture + infer.

Agent doesn't pick the path — it calls `observe()`. The body picks the best sensor.

**Synapse does NOT include:**

- Goal-planning, MCTS, GOAP, skill libraries, or hierarchical decomposition (agent handles via tool-use loop)
- Large prediction model, reward model, or learning loop (agent does this in-context)
- Inner LLM (model lives outside; pure infrastructure)
- Unsupported process-manipulation, device-identity, or scaled unattended account-operation features

---

## Why one system for both

Shared load-bearing primitives:

| Primitive | Computer-use | Game-control |
|---|---|---|
| Zero-copy GPU frame capture | Yes (for canvas/video/Electron a11y holes) | Yes (primary perception) |
| Accessibility tree (UIA) walk + event hook | Yes (primary perception) | Sometimes (UI overlays, menus) |
| Structured semantic event stream | Yes (focus / mutation / file change) | Yes (entity appeared / HUD change / audio cue) |
| Keyboard / mouse SendInput | Yes | Yes |
| Virtual controller (ViGEm) | Rarely | Yes |
| Hardware HID (RP2040 gateway) | Rarely | Yes for physical-input rigs |
| Aim curves / smooth cursor motion | Optional | Critical for games |
| Sub-frame reflex runtime | Helpful (auto-dismiss popups) | Critical (frame-perfect inputs) |
| OCR fallback | Yes (when a11y is sparse) | Yes (HUD readouts) |
| Per-app/per-game profile system | Yes | Yes |
| Token-efficient `observe()` JSON | Yes | Yes |

Two separate products would duplicate ~90% of engineering. **Synapse ships them once.**

## Strategic moat: profile registry + audit data

Issue #454 is the binding product decision: the profile-registry / audit-data
network effect is P1 strategic architecture. Profiles encode app/game operating
knowledge; audit rows prove which profile decisions worked on this configured
host. The compounding loop is profile used -> audit evidence captured ->
quality/compatibility learned -> profile improved -> registry distributes a
better profile -> more evidence.

This loop must be designed as local-first and consent-gated. The physical
sources of truth are the profile TOML files, future registry index/package
files, RocksDB `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `CF_EVENTS`,
`CF_OBSERVATIONS`, `CF_SESSIONS`, and `CF_PROFILES` quality rows,
consent/export bundles, and MCP readbacks such as `profile_list`,
`profile_quality_refresh`, `storage_inspect`, and future registry/audit tools.
Manual FSV must trigger the real runtime surface and then read these stores
directly; scripts, tests, benchmarks, GitHub Actions, and CI are never FSV.

---

## Read order

| # | Doc | Read when |
|---|---|---|
| README | this file | Always start here |
| 00 | [`00_vision_and_scope.md`](00_vision_and_scope.md) | First. Mission, users, non-goals. |
| 01 | [`01_architecture.md`](01_architecture.md) | Moving parts. Processes, threads, Rust workspace. |
| 02 | [`02_perception.md`](02_perception.md) | Eyes/ears. Capture, detection, a11y, OCR, audio, events. |
| 03 | [`03_action.md`](03_action.md) | Hands. Mouse/kbd/controller/HID. |
| 04 | [`04_reflex_runtime.md`](04_reflex_runtime.md) | Sub-frame reactive controllers. Event bus. |
| 05 | [`05_mcp_tool_surface.md`](05_mcp_tool_surface.md) | Public API. Every tool, parameter, return, error. |
| 06 | [`06_data_schemas.md`](06_data_schemas.md) | Rust structs, JSON envelopes, event types, error codes. |
| 07 | [`07_storage_and_profiles.md`](07_storage_and_profiles.md) | RocksDB schema, runtime files, profile system. |
| 08 | [`08_supported_use_policy.md`](08_supported_use_policy.md) | What we will/won't ship, supported contexts, operator acknowledgments. |
| 09 | [`09_hardware_hid_gateway.md`](09_hardware_hid_gateway.md) | Pi Pico HID firmware + serial protocol + host driver. |
| 10 | [`10_performance_budget.md`](10_performance_budget.md) | Latency targets, profiling, optimization rules. |
| 11 | [`11_security_and_safety.md`](11_security_and_safety.md) | Threat model, permissions, redaction, kill switches. |
| 12 | [`12_observability.md`](12_observability.md) | Logging, tracing, metrics, debug overlay, replay tool. |
| 13 | [`13_testing_strategy.md`](13_testing_strategy.md) | Unit/integration/E2E, fixtures, manual FSV, perf regression. |
| 14 | [`14_build_and_packaging.md`](14_build_and_packaging.md) | Workspace, deps, profiles, installer, signing. |
| 15 | [`15_roadmap_and_milestones.md`](15_roadmap_and_milestones.md) | M0-M5 phases, scope per milestone, demo criteria. |
| 16 | [`16_open_questions.md`](16_open_questions.md) | Unresolved decisions, ADRs needed. |
| 17 | [`17_research_appendix.md`](17_research_appendix.md) | Web research, comparable projects, references. |
| 18 | [`18_luanti_benchmark.md`](18_luanti_benchmark.md) | Local Luanti / Minetest Game benchmark install, world fixture, and provenance. |
| 19 | [`19_luanti_manual_fsv_runbook.md`](19_luanti_manual_fsv_runbook.md) | Manual whole-system FSV runbook for the Luanti benchmark. |
| 20 | [`20_profile_registry_governance.md`](20_profile_registry_governance.md) | Profile/audit contribution rights, licenses, attribution, provenance, and revocation governance. |
| 21 | [`21_profile_registry_protocol.md`](21_profile_registry_protocol.md) | Optional shared registry protocol, local source config, moderation state, update metadata, and contribution submission boundary. |
| 22 | [`22_profile_registry_data_model.md`](22_profile_registry_data_model.md) | Local-first profile registry data model, `CF_PROFILES`/`CF_KV` row namespaces, and install/register edge rules. |
| 23 | [`23_profile_package_manifest.md`](23_profile_package_manifest.md) | Profile package manifest schema, provenance, compatibility matrix, permissions, and fail-closed validation rules. |
| 24 | [`24_curated_starter_registry.md`](24_curated_starter_registry.md) | Curated starter profile registry seed set, per-target backlog, and registry row SoT. |
| 25 | [`25_mcp_runtime_fsv_path.md`](25_mcp_runtime_fsv_path.md) | Reliable MCP runtime path for manual FSV, direct chat `Transport closed` boundary, and repo-owned HTTP/stdout SoTs. |

---

## One-line system summary

A Rust MCP server that exposes structured desktop and game state as low-token JSON, accepts high-level action intents (click, type, aim, press, drag, combo), runs sub-frame reflexive controllers so model latency never costs a frame, and pipes everything through a single `synapse-mcp` binary the agent connects to over stdio or Streamable HTTP.

---

## Architecture sketch

```
┌─────────────────────────────────────────────────────────────────────┐
│  AI Agent (Claude / Codex / Cursor / custom runner) — the BRAIN     │
│  - Sets goals, plans, composes skills, adapts                       │
│  - Speaks MCP JSON-RPC over stdio or Streamable HTTP                │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ MCP request/response (slow loop ~1-3 Hz)
                               │ MCP notifications (push events, SSE)
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│  synapse-mcp (Rust binary) — the BODY                               │
│                                                                     │
│  ┌────────── Perception ──────────┐                                 │
│  │ ┌─ A11y path (UIA, CDP) ────┐  │ ──► observe() returns       │
│  │ │  tree walk + event hook  │  │       structured app state  │
│  │ └─ Pixel path (capture+CNN)┘  │                                 │
│  │   GPU frame → YOLO/ConvNeXt  │                                 │
│  │ ┌─ Audio (WASAPI loopback) ─┐  │                                 │
│  │ │  STT + spatial direction  │  │                                 │
│  │ └─ HUD OCR + template match ┘  │                                 │
│  └────────────────┬───────────────┘                                 │
│                   │                                                  │
│                   ▼ structured events to bus                         │
│  ┌────────── Reflex Runtime ──────┐                                 │
│  │  aim_track / on_event / combo │  ──► fast loop 60-1000 Hz       │
│  │  scheduler @ 1ms tick         │                                 │
│  └────────────────┬───────────────┘                                 │
│                   │                                                  │
│                   ▼                                                  │
│  ┌────────── Action ─────────────┐                                  │
│  │  SendInput  │  ViGEm  │  HID  │  ──► OS / virtual controller /   │
│  │  Interception driver  (opt)   │       USB serial → RP2040 board │
│  └────────────────┬───────────────┘                                 │
│                   │                                                  │
│  ┌────────────────▼─────────────────────────────────────────────┐   │
│  │  Event bus + RocksDB + profiles + tracing telemetry         │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
                               │
                               ▼
                Windows OS + GPU + foreground apps and games
```

Slow loop (model → MCP → response) runs at human-decision rate. Fast loop (reflex runtime) at frame rate. Decoupled by event bus.

---

## Performance targets (binding)

| Stage | Target p99 |
|---|---|
| Frame capture (zero-copy GPU surface) | ≤ 3 ms |
| Detection inference (small CNN on 5090-class GPU) | ≤ 8 ms |
| UIA tree snapshot for focused window | ≤ 10 ms |
| Full `observe()` response from request to reply | ≤ 30 ms |
| Event push from underlying frame/UIA event to subscriber | ≤ 50 ms |
| `act_aim_at` start-of-motion latency | ≤ 5 ms |
| `act_press` to electrical signal on USB | ≤ 2 ms (software) / ≤ 4 ms (hardware HID) |
| Reflex `on_event` action emission | ≤ 5 ms from event |
| MCP idle-tick CPU usage | ≤ 1% on one core |
| Steady-state VRAM when models loaded | ≤ 2 GB |

Detailed budget and profiling discipline: `10_performance_budget.md`.

---

## Quick start (target M3+ DX)

```powershell
# One-time prerequisites
winget install Nefarius.ViGEmBus       # virtual controller driver; use the GUI installer on Win11

# Install Synapse
cargo install --git https://github.com/ChrisRoyse/Synapse synapse-mcp

# Stdio mode (for Claude Desktop / Codex CLI etc.)
synapse-mcp --mode stdio

# Streamable HTTP mode (for remote / multi-client agents)
synapse-mcp --mode http --bind 127.0.0.1:7700

# Optional: flash an RP2040 board for hardware HID
cargo run -p synapse-hid-host -- flash --device COM7
```

Win11 ViGEmBus note: the Nefarius 1.22.0 installer works by operator GUI
clickthrough on the developer machine. Do not rely on `/SILENT`,
`/VERYSILENT`, or installer extraction flags for current M2 setup; the Windows
FSV finding in issue #229 showed those unattended paths exit before producing a
driver install or log.

Configure your agent client to launch `synapse-mcp` as an MCP server. Agent gains every tool in `05_mcp_tool_surface.md`.

---

## Repository layout

```
synapse/
├── Cargo.toml                          # workspace root
├── README.md
├── LICENSE-MIT, LICENSE-APACHE
├── docs/                               # this PRD lives here
├── crates/
│   ├── synapse-mcp/                    # binary: MCP server entry
│   ├── synapse-core/                   # shared types, error codes, constants
│   ├── synapse-capture/                # GPU frame capture (windows-capture wrapper)
│   ├── synapse-a11y/                   # UIA tree walk + WinEvent hook + CDP client
│   ├── synapse-perception/             # detection, OCR, HUD, event derivation
│   ├── synapse-audio/                  # WASAPI loopback + STT + spatial direction
│   ├── synapse-action/                 # input emit (kbd/mouse/pad/HID)
│   ├── synapse-reflex/                 # sub-frame reactive runtime
│   ├── synapse-storage/                # RocksDB wrapper + CFs
│   ├── synapse-profiles/               # per-app/per-game profile loader
│   ├── synapse-hid-host/               # Rust serial driver for hardware HID gateway
│   ├── synapse-models/                 # ONNX runtime wrappers, model loader
│   ├── synapse-telemetry/              # tracing + metrics + replay log
│   └── synapse-test-utils/             # shared test helpers
├── firmware/
│   └── pico-hid/                       # RP2040 HID gateway (Rust, embassy-rs, no_std)
├── models/                             # bundled ONNX models (small set, downloaded on demand)
├── profiles/                           # bundled app + game profiles (community-extendable)
├── scripts/                            # build, signing, install (PowerShell + Rust)
└── tests/
    ├── e2e/
    └── fixtures/
```

Crate boundaries and dep graph in `01_architecture.md`. Build details in `14_build_and_packaging.md`.

---

## Out of scope (explicit non-goals)

1. **Unsupported process manipulation or scaled unattended account operation.** Synapse is for local computer-use automation, accessibility, QA, research, simulation rigs, and single-player/sanctioned game-control environments.
2. **Goal/planning/skill libraries.** Agent via tool-use loop.
3. **Inner LLM.** Optional small vision models (YOLO-nano, ConvNeXt-tiny) only.
4. **Cross-platform v1.** Windows first. Linux/macOS v2.
5. **General-purpose RPA.** Web/SaaS form-filling is a side-effect, not target.
6. **Reverse-engineering proprietary game protocols.** RAM reads / packet inspection are out of v1 unless an application exposes an official local API for that purpose.

---

## Project status

| Phase | Status |
|---|---|
| PRD | Active drafting |
| M0 — bootstrap | DONE (tag `v0.1.0-m0`, 2026-05-23) |
| M1 — perception MVP (a11y + capture) | In progress (active phase) |
| M2 — action MVP (kbd/mouse/pad) | Not started |
| M3 — reflex + MCP surface | Not started |
| M4 — hardware HID + first game profile | Not started |
| M5 — production-ready, 5+ app/game profiles | Not started |

See `15_roadmap_and_milestones.md`.

---

## Authoring rules

- Files ≤ 500 lines, functions ≤ 30 lines, cyclomatic ≤ 10.
- `#![forbid(unsafe_code)]` everywhere except `synapse-capture` (DirectX FFI) and `synapse-hid-host` (serial / OS handle).
- All errors carry `SCREAMING_SNAKE_CASE` codes via `thiserror`. No `anyhow` in library crates.
- Public APIs and CF names are `pub const` — no magic strings.
- `tracing` for everything. `println!` is a code-review rejection.
- No silent successes. Tool that didn't do the work returns an error code.
- No mocks in tests that gate completion. Real captures, input, RocksDB.
- Manual FSV is done by the agent with direct source-of-truth readback; tests,
  scripts, benchmarks, GitHub Actions, and CI are supporting evidence only.
- Pre-production: schema changes = wipe-and-rebuild, no migration shims.

---

## Authoritative summary

**Synapse is the Rust MCP server that lets any AI agent see, hear, and act on a Windows machine — covering both productivity computer-use (Office, IDEs, browsers, terminals) and real-time video games — by auto-selecting between accessibility-tree perception and GPU-frame-capture perception, exposing structured state and semantic events as token-efficient JSON, accepting high-level action intents that the body compiles to frame-accurate input, and running sub-frame reflexive controllers so model latency never costs a frame.**
