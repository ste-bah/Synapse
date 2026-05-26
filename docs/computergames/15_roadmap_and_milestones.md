# 15 — Roadmap and Milestones

## 1. Milestone overview

| Milestone | Theme | Effort (solo) |
|---|---|---|
| **M0** | Bootstrap — workspace, MCP loopback, local checks | 1 week |
| **M1** | Perception MVP — capture + UIA + observe() | 2-3 weeks |
| **M2** | Action MVP — kbd/mouse/pad + clipboard | 2 weeks |
| **M3** | Reflex + MCP surface — tools, push events, profiles | 2-3 weeks |
| **M4** | Hardware HID + first game profile | 2-3 weeks |
| **M5** | Production polish — installer, docs, profiles, perf | 3-4 weeks |

~14 weeks solo full-time to v1.0; ~8 weeks with two engineers. Each milestone has a hard demo criterion. No demo, no milestone.

---

## 2. M0 — Bootstrap (1 week)

**Goal:** empty repo to "MCP server returning hardcoded data."

### Scope

- Cargo workspace, 15 crates (most stubs)
- `synapse-core` types (`Backend`, `Point`, `Rect`, M0 error codes)
- `synapse-mcp` binary with `rmcp`
- One tool: `health` returns `{"ok": true, "version": "..."}`
- stdio transport with Claude Desktop / Codex CLI
- `tracing` JSON file logger
- Local supporting checks: `cargo fmt`, `cargo clippy`, `cargo test`
- README "Hello, Synapse"
- `synapse-test-utils` with custom MCP client

### Out of scope

Real perception/action, storage (`()` placeholder), profiles, models.

### Demo criterion

Claude Desktop calls `health` via Synapse MCP, sees `{"ok": true}`.

### Files created

```
Cargo.toml, deny.toml, .gitignore
LICENSE-MIT, LICENSE-APACHE
README.md
docs/                                  (this PRD)
crates/synapse-core/
crates/synapse-mcp/
crates/synapse-test-utils/
crates/synapse-storage/                (stub)
crates/synapse-perception/             (stub)
crates/synapse-action/                 (stub)
crates/synapse-reflex/                 (stub)
crates/synapse-capture/                (stub)
crates/synapse-a11y/                   (stub)
crates/synapse-audio/                  (stub)
crates/synapse-profiles/               (stub)
crates/synapse-hid-host/               (stub)
crates/synapse-models/                 (stub)
crates/synapse-telemetry/
crates/synapse-overlay/                (stub)
.github/workflows/ci.yml
scripts/release/ (skeleton)
```

---

## 3. M1 — Perception MVP (2-3 weeks)

**Goal:** describe any focused window as structured JSON.

### Scope

- `synapse-capture`: `windows-capture`; emit `CapturedFrame` over crossbeam channel
- `synapse-a11y`: UIA tree walker (depth-limited snapshot); WinEvent hook (foreground, focus, value, structure); small UIA cache (focused element)
- `synapse-perception`: stub detection (empty unless model loaded); WinRT OCR wrapper; `Observation` assembler
- `synapse-models`: minimum ONNX loader; YOLOv10n via `ort`
- `synapse-mcp` adds: `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`, `health`
- Coordinate transforms (per-monitor DPI)

### Out of scope

Audio, HUD profiles, reflexes, action, replay log.

### Demo criterion

Notepad focused. `observe()` returns `foreground.process_name = "notepad.exe"`, `focused.role = "Edit"`, editor bounding rect. Round trip ≤ 50 ms.

### Risk areas

- UIA cross-process COM marshaling slow; cache request batching day one
- DirectX texture lifetime; time on `Drop`/`RAII` correctness
- `ort` + DirectML setup on clean Windows has paperwork (MSVC redist)

---

## 4. M2 — Action MVP (2 weeks)

**Goal:** drive any app's input.

### Scope

- `synapse-action`: software backend via `enigo` + direct `windows-rs`; action serialization actor (single mpsc emitter); `ReleaseAll` safety on shutdown/panic; held-input tracking + auto-release timeout; ViGEm backend via `vigem-client`; coordinate transforms for screen/window/element clicks; UIA `InvokePattern` semantic invoke
- `synapse-mcp` adds: `act_click`, `act_type`, `act_press`, `act_aim`, `act_drag`, `act_scroll`, `act_pad`, `act_clipboard`, `release_all`
- Aim curves: `Instant`, `Linear`, `EaseInOut`, `Bezier`, `Natural`
- Keystroke dynamics: `Burst`, `Linear`, `Natural`

### Out of scope

Hardware HID, combos (M3), run-shell/launch.

### Demo criterion

`act_click(element_id=<Notepad editor>)`, `act_type("Hello")`, `act_press(["ctrl","s"])`; save dialog appears; `observe()` shows it.

### Risk areas

- ViGEm needs ViGEmBus installed on the operator's configured Windows host;
  M2 ships from manual FSV on that host, not from CI runner coverage
- `Natural` curve takes iteration; default `EaseInOut` until M5

---

## 5. M3 — Reflex + MCP Surface (2-3 weeks)

**Goal:** push-event subscriptions, reflexes, profiles, full tool surface.

### Scope

- `synapse-reflex`: event bus (`crossbeam` broadcast); reflex scheduler on dedicated time-critical thread; five reflex kinds (`aim_track`, `hold_move`, `hold_button`, `combo`, `on_event`); audit log to `CF_REFLEX_AUDIT`
- `synapse-storage`: RocksDB integration; CF set per `07_storage_and_profiles.md`; compaction filters for TTL; GC with soft/hard caps; disk pressure responder
- `synapse-profiles`: TOML loader; hot-reload via `notify`; detection (exe + title match); bundled `notepad`, `vscode`, `chrome`, `terminal`
- `synapse-mcp` adds: `subscribe`, `subscribe_cancel`, `reflex_register`, `reflex_cancel`, `reflex_list`, `reflex_history`, `profile_list`, `profile_activate`, `replay_record`, `audio_tail`, `audio_transcribe`
- Streamable HTTP transport (alongside stdio); push notifications via SSE
- `synapse-audio` MVP: WASAPI loopback + simple direction + Whisper-tiny STT

### Out of scope

Hardware HID, game profiles, debug overlay, VLM `describe`.

### Demo criterion

Agent registers `on_event` reflex: "when Save dialog appears, type path + Enter." Triggers via `act_press(["ctrl","s"])`; reflex fires; no intervention until saved.

### Risk areas

- Time-critical thread scheduling on Windows; debug jitter
- Profile hot-reload vs active state ordering
- Streamable HTTP / SSE re-connect semantics
- RocksDB Windows reliability; M3 uses RocksDB only per ADR-0002

---

## 6. M4 — Hardware HID + First Game Profile (2-3 weeks)

**Goal:** play one game end-to-end, including via hardware HID.

### Scope

- `synapse-hid-host`: serial driver; identity handshake; frame protocol with CRC, ACK/NAK, watchdog; reconnect logic
- `firmware/pico-hid/`: RP2040 firmware in Rust with `embassy-rp`; USB HID composite (mouse + keyboard + pad) + CDC ACM serial; watchdog, protocol parser, LED status, `.uf2` build pipeline
- `synapse-mcp` adds: `act_combo` (reflex scheduler), `act_run_shell` (gated), `act_launch` (gated), `hid identify`, `hid flash`
- First game profile: `minecraft.java` — HUD (hp_hearts, hunger, xp); keymap with Minecraft defaults; detection YOLOv10n_general (no MC fine-tune); `event_extensions` (`creeper_nearby`, `low_hp`)
- Supported-use policy enforcement (`08`): profile `use_scope` metadata, backend permission gating

### Out of scope

Multiple game profiles (M5), VLM `describe`, debug overlay, installer.

### Demo criterion

Agent + Minecraft. `observe()` returns HP and visible entities. Walks "find tree, break, plank, workbench" via `act_press`, `act_aim`, reflexes for `auto_attack_low_hp`. Runs 5 min without intervention.

Bonus: same demo via hardware HID (`--hardware-hid auto`).

### Risk areas

- Detection accuracy on Minecraft (specialty fine-tune may be needed)
- HUD OCR for hearts/hunger via template-match; carefully cropped assets
- Hardware HID latency under sustained load; benchmark + tune

---

## 7. M5 — Production Polish (3-4 weeks)

**Goal:** v1.0 ship-ready.

### Scope

- Installer (`SynapseSetup.msi`) via `wix-installer`
- Code signing (self-signed initially; project cert when funded)
- 5+ additional bundled profiles: `factorio`, `discord` / `slack`, `file_explorer`, `<one_fps>` (TBD, free game), `roblox_studio`
- Debug overlay (`synapse-overlay`)
- VLM-based `describe` (Florence-2-base ONNX)
- Full Grafana dashboards
- Complete docs (this PRD + user-facing `USER_GUIDE.md`)
- Stable schema (v1 locked; future via migration / DB wipe)
- Performance budgets in `10_performance_budget.md` met on reference machine
- Soak test passes 8 hours clean
- Crash dump infrastructure
- `synapse-mcp setup` wizard
- Tray icon; license + token management
- Public release on GitHub Releases + crates.io; winget submission

### Demo criterion

Fresh Windows 11, no Synapse. Operator runs `synapse setup`, follows wizard, opens Claude Desktop. Agent:

1. Open VS Code, write small Rust file
2. `cargo build` via terminal
3. Switch to Chrome, search "Synapse MCP project," read result
4. Switch to Minecraft, play 2 minutes
5. Switch to music player, control playback

No screenshots; total token cost < 30K.

---

## 8. Post-v1

v1 ships at M5. v2+ priorities:

### v1.x patches

- Per-game fine-tuned detection models (`yolov10n_minecraft`, `yolov10n_factorio`)
- `Natural` aim curve improvements from feedback
- More bundled profiles via community

### v2 horizons

| Feature | Effort |
|---|---|
| Linux support (Wayland + AT-SPI) | ~6 weeks |
| macOS support (AX + ScreenCaptureKit + native input) | ~6 weeks |
| Cross-platform CDP (already half via `chromiumoxide`) | ~1 week |
| Per-game RAM hooks for sanctioned games (Minecraft mod API, KSP plugin) | ~2 weeks/game |
| Visual replay viewer (web app) | ~4 weeks |
| Profile marketplace (community-contributed with signing) | ~4 weeks |
| Steam Audio for spatial (HRTF replacement) | ~2 weeks |
| Sub-ms aim via PIO USB host on RP2040 (pass-through + corrections) | ~3 weeks |
| Browser DOM-only mode (structured-DOM RPA; no a11y/pixels) | ~2 weeks |

Not committed; v2 roadmap decided after v1 ships.

---

## 9. Risks and mitigations

| Milestone | Risk | Mitigation |
|---|---|---|
| M0 | rmcp API churn | Pin version; vet dep PRs |
| M1 | UIA performance | Cache request batching day one; fall back to depth-1 |
| M1 | DirectML on AMD/Intel | CPU fallback for detection; warn at startup if no GPU EP |
| M2 | ViGEm install friction | Document Win11 GUI clickthrough; auto-detect; if ViGEm is absent on the configured host, acquire/install it through local workflows before treating gamepad work as blocked |
| M3 | Time-critical thread jitter | Multimedia timer; fall back to `tokio::time` 2 ms tick if no MMCSS |
| M3 | RocksDB Windows hiccups | pinned RocksDB version; alternate backend requires future ADR |
| M4 | RP2040 firmware debug pain | Loopback build feature off-target; configured-host Pico checks |
| M4 | Minecraft detection accuracy | Mark accuracy honestly; commit to fine-tune in v1.x |
| M5 | MSI signing cert | Self-sign at v1.0; document SmartScreen warning; cert acquisition separate workstream |
| M5 | VLM bundle size | VLM optional download; `describe` returns `MODEL_NOT_LOADED` until downloaded |

---

## 10. Acceptance criteria

Release shippable when all true:

1. M0–M5 demos pass
2. Performance budgets in `10_performance_budget.md` met on reference machine (RTX 3060 + 8-core CPU)
3. Local configured-host checks and manual FSV green on the release candidate (no flakes in the exercised gates)
4. Soak test passes 8 hours
5. Manual test plan in `13_testing_strategy.md` §15 signed off
6. PRD internally consistent (no broken cross-refs)
7. License compliance clean (`cargo deny check`)
8. No `unsafe` outside documented allowed crates
9. No `unwrap()` outside tests (`#[deny(clippy::unwrap_used)]`)
10. Crash dumps verified on intentional panics

---

## 11. Out-of-bound items (not scheduled at v1)

- AI-driven profile authoring
- Cloud-hosted Synapse-as-a-service
- Multi-machine orchestration
- Mobile MCP clients driving Synapse remotely
- Sandbox / VM auto-provisioning
- Encrypted replay exports
- Real-time co-pilot (agent + human sharing input)

Fine v2+ ideas; cleanly outside v1.

---

## 12. The v1 promise

Operator gets:

- Signed Windows installer on PATH
- First-run wizard ≤ 5 minutes
- MCP server compatible with every major agent client
- ≤ 30 ms p99 `observe()` for productivity apps
- Real-time game support for ≥ 2 single-player titles
- Documented hardware HID path for accessibility / research
- Complete PRD + user guide + reference docs
- Active GitHub Issues + Discussions community
- v1.x roadmap

The contract.

---

## 13. What this doc does NOT cover

- Issue tracker / project board (GitHub Projects)
- Sprint planning / iteration cadence
- Commercial roadmap
- Specific demo-game choices (finalized closer to M4)
