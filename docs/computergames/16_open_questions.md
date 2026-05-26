# 16 — Open Questions

Decisions deliberately not made in the PRD. Each entry: description, trade-off, current default, decision target.

When resolved, replace entry with "→ decided in ADR-NNN". ADRs live in `docs/adr/NNN-title.md` (post-v1 if needed).

---

## OQ-001 — Sled vs RocksDB as primary — DECIDED 2026-05-24

→ decided in ADR-0002.

**Decision.** RocksDB is the only M3 storage backend. The unused sled escape
valve was removed because it had no implementation and pulled vulnerable /
unmaintained transitive crates into `Cargo.lock`.

**Future change.** A fallback backend requires a new issue, implemented adapter,
maintained dependency graph, and manual source-of-truth verification on the
configured Windows host.

---

## OQ-002 — Streamable HTTP session vs stateless

**Q.** Stateful (`Mcp-Session-Id`) or stateless (per-request capability tokens) HTTP sessions?

**Trade-off.** Stateful: simpler, matches Claude Desktop, reflexes/subscriptions across calls. Stateless: scalable, harder to misuse, fits SEP-1442.

**Default.** Stateful with `Mcp-Session-Id`. Reflexes and subscriptions are session-scoped.

**Target.** v2 if multi-tenant deployment; v1 stays stateful.

---

## OQ-003 — Detection model default

**Q.** YOLOv10n vs RT-DETR-s as default?

**Trade-off.** YOLOv10n: faster (~5 ms on 5090), smaller (~6 MB), higher jitter. RT-DETR-s: more stable across frames (~10 ms), bigger (~80 MB), better small-object recall.

**Default.** YOLOv10n.

**Target.** M4. Benchmark both on Minecraft entity detection. Switch if RT-DETR-s materially improves task success.

---

## OQ-004 — Aim curve default for productivity — DECIDED 2026-05-22

**Decision.** **`Natural` (with `AimNaturalParams::FAST` preset) is the default everywhere — productivity profiles, game profiles, all aim styles. `Instant` is never a default; it stays in the enum solely for explicit caller opt-in (e.g., test harnesses requiring pixel-perfect positioning).** No curve defaults to a discontinuous jump.

**Rationale.** A single doctrine — smooth + natural + very fast — is simpler than per-profile branching, avoids discontinuous cursor jumps, and preserves replay readability at zero perf cost when params are tuned `FAST` (50 ms total travel for `Snap`). See `03_action.md` §6 (`AimNaturalParams::FAST` preset).

**Consequence.**

- All bundled profiles set `mouse_curve_default = "natural"` and `keyboard_dynamics_default = "natural"`
- `act_click` default `duration_ms = 50` (was 80); `act_aim` `Snap` 50 ms; `Flick` 35 ms
- `KeystrokeDynamics::Burst` retained in enum; never a default
- Per-tool perf targets in `10_performance_budget.md` §12 unchanged — `Natural::FAST` fits within budgets
- Impplan `docs/impplan/03_m2_action_mvp.md` ships `Natural::FAST` defaults at first introduction (no later flip)

---

## OQ-005 — Reflex priority arithmetic — DECIDED 2026-05-25

→ decided in ADR-0004.

**Decision.** Reflex priority is a `u32` where lower numbers win. The default
priority is `100`. Ties are broken by registration order, with newer
registrations winning. A loser contending for the same cursor, key, mouse
button, or gamepad resource for 2 seconds becomes `Starved`; Synapse emits one
`reflex_starved` event and writes one `REFLEX_STARVED` audit row per starvation
interval.

---

## OQ-006 — Permission model: profile-level vs session-level

**Q.** Permissions (`allow_launch`, `allow_shell`, `allow_hardware_hid`) per-profile, per-session, or global?

**Trade-off.** Per-profile: per-app posture, clean but verbose. Per-session: agent caller scopes itself. Global: simplest.

**Default.** Global (CLI/config) + per-profile overrides for `backends.*`. No per-session granularity.

**Target.** v2 if multi-session deployment.

---

## OQ-007 — Profile signing

**Q.** Sign bundled profiles and verify at load? Community profiles require community-key signature?

**Trade-off.** Signing protects against tampering, adds supply-chain integrity. Friction for community contributors.

**Default.** No signing at v1. Profiles are plain TOML, no code execution, low risk.

**Target.** Post-v1 (profile marketplace). v2 likely brings optional signing.

---

## OQ-008 — Bundled VLM for `describe`

**Q.** Bundle Florence-2-base in installer for first-run `describe`?

**Trade-off.** Bundled: always available, no first-use latency, +500 MB install. Download-on-demand: smaller install, first call slow.

**Default.** Download-on-demand. `describe` returns `MODEL_NOT_LOADED` until `models import` or first-call triggers download.

**Target.** v1.x. Switch to bundled if operators consistently confused by first-call slow path.

---

## OQ-009 — Maximum elements in `observe()` response

**Q.** Right `max_elements` default?

**Trade-off.** Higher: more context, more tokens. Lower: smaller responses, agent may miss elements.

**Default.** 60. With depth=2 covers most focused-window contexts, stays under ~6 KB JSON.

**Target.** M5 telemetry. Raise if `elements_truncated` is true in >20% of calls. Lower + add `expand(slot=...)` if responses too big.

---

## OQ-010 — A11y CDP integration depth

**Q.** Auto-attach to every Chromium browser, or require explicit `cdp_attach`?

**Trade-off.** Auto: seamless, needs `--remote-debugging-port`. Explicit: agent must know to attach.

**Default.** Auto-attempt when foreground is known Chromium browser AND debugging port configured. Surface `CDP_UNREACHABLE` if not — no silent failure.

**Target.** M3 ship state; revise if explicit-port requirement painful.

---

## OQ-011 — Hardware HID firmware language

**Q.** Rust+embassy on RP2040 or C+TinyUSB?

**Trade-off.** Rust+embassy: stack consistency, type safety; larger flash footprint, slower iteration on USB stack issues. C+TinyUSB: smaller, more mature USB stack, faster external-contributor onboarding.

**Default.** Rust+embassy. All-Rust project; overhead worth consistency.

**Target.** Locked unless embassy USB stack bugs block M4 demo.

---

## OQ-012 — Multi-monitor capture — DECIDED 2026-05-25

→ decided in ADR-0005.

**Decision.** Synapse has one active capture target per session. The default is
the primary monitor. Agents switch targets explicitly through
`set_capture_target`; Synapse does not stitch the virtual desktop and does not
run concurrent per-monitor captures in M3.

---

## OQ-013 — Reflex `aim_track` smoothing under jitter

**Q.** How aggressively does aim_track follow detection track jitter (e.g., x=820 → 824 → 818 → 822)?

**Trade-off.** Every micro-jitter: mouse hunting. Smoothing: lag.

**Default.** EMA `alpha = 0.7` applied to track position before aim-error calc. Configurable in reflex params.

**Target.** M4 game testing. Tune from gameplay footage.

---

## OQ-014 — Whisper-tiny vs Whisper-base STT

**Q.** Default Whisper-tiny (~40 MB, faster) or Whisper-base (~140 MB, more accurate)?

**Trade-off.** Speed vs accuracy. STT for "what did NPC say" / "voice chat" — neither latency-critical but accuracy matters.

**Default.** Whisper-tiny-int8. Operators can `models import whisper-base.onnx`.

**Target.** M5 feedback. May add Whisper-base to bundle if disk budget permits.

---

## OQ-015 — Profile match precedence — DECIDED 2026-05-25

→ decided in ADR-0006.

**Decision.** Automatic foreground profile resolution ranks matches by
`exe > title_regex > steam_appid > window_class`. Same-rank conflicts are
resolved by newer profile file mtime, then deterministic source path/profile id
tie-breakers. Manual `profile_activate` remains an explicit override.

---

## OQ-016 — Action coalescing

**Q.** If agent fires `MouseMoveRelative(1, 0)` 100 times rapidly, coalesce into `MouseMoveRelative(100, 0)`?

**Trade-off.** Coalescing reduces USB poll pressure on hardware HID. Not coalescing preserves exact timing.

**Default.** No coalescing for software (cheap anyway). Coalescing for hardware when target poll interval would be missed (deferred ≤ 2 ms of pending small moves merge).

**Target.** M4 hardware HID testing. Tune window.

---

## OQ-017 — Disk pressure thresholds

**Q.** Are 2 GB / 1 GB / 500 MB / 200 MB the right free-disk thresholds for pressure levels 1-4?

**Trade-off.** Higher: wastes free disk on small SSDs. Lower: risk of running out.

**Default.** Listed thresholds. Operator can override via config.

**Target.** v1.x telemetry from operators on small drives.

---

## OQ-018 — Replay export format

**Q.** ZIP+JSONL+frames vs SQLite vs custom binary?

**Trade-off.** ZIP: portable, inspectable. SQLite: queryable. Custom binary: compact, opaque.

**Default.** ZIP + JSONL + WebP frames.

**Target.** Locked unless replay viewer (v2) needs different format.

---

## OQ-019 — Telemetry endpoint for debug vs production

**Q.** Debug telemetry (per-event traces) on same `/metrics` endpoint as ops, or split?

**Trade-off.** Single: simpler. Split: production ops not spammed.

**Default.** Single endpoint; `metrics_level` config (`production` | `debug`) controls verbosity. Default `production`.

**Target.** M5. May split if endpoint size complaints.

---

## OQ-020 — Expose raw frame access?

**Q.** Agent can request raw screenshot via `act_screenshot_once`. Violates "structure over pixels" but sometimes needed (e.g., `describe` fallback). Expose or force `describe`?

**Trade-off.** Exposing: escape hatch agents need. Hiding: forces structure, may break workflows.

**Default.** Expose `game_screenshot_once` (renamed from `act_screenshot_once`) as escape hatch. Documented "use sparingly."

**Target.** M3. M5 telemetry should show <5% of agent turns calling it.

---

## OQ-021 — Audio direction estimate vs full spatial

**Q.** Naive L/R energy + cross-correlation, or HRTF via Steam Audio?

**Trade-off.** Naive: cheap, accurate to ~30° azimuth. Steam Audio: precise, big dep.

**Default.** Naive at v1. Steam Audio is v2.

**Target.** v1.x if audio-direction reflexes popular and accuracy complaint.

---

## OQ-022 — Reflex `on_event` recursion guard — DECIDED 2026-05-25

→ decided in ADR-0003.

**Decision.** Chained `on_event` reflexes are allowed, but the scheduler permits
at most four successful `on_event` firings per tick across all active reflexes.
A fifth same-tick match is skipped, the remaining event-driven firings wait for
the next tick, and `REFLEX_RECURSION_LIMIT` is emitted/audited.

---

## OQ-023 — Stability of element_id across UIA snapshots

**Q.** UIA `RuntimeId` documented as stable for element lifetime, but can change after structural mutations. Trust it or maintain own ID layer?

**Trade-off.** Trusting saves work. Own layer: more reliable, more code.

**Default.** Composite ID (`<hwnd>:<runtime_id_hex>`) is public identifier. Re-resolve on every action via runtime ID; fall back to lookup by name+role+position. Agent may need re-`observe()` if elements churn.

**Target.** M2 testing. Build wrapper if stability issues surface.

---

## OQ-024 — Tokenization budget enforcement

**Q.** `observe()` proactively trim to token budget (e.g., 1500), or return all and let client trim?

**Trade-off.** Server-side: reliable. Client: flexible.

**Default.** Server-side trim with explicit `Observation.diagnostics.elements_truncated` flag.

**Target.** M3. May add `max_tokens` param to `observe()`.

---

## OQ-025 — Bundled detection model license

**Q.** YOLOv8/YOLOv10/YOLOv11 are AGPL (Ultralytics). Can we bundle weights?

**Trade-off.** AGPL incompatible with MIT/Apache. Bundling forbidden. Local
acquisition is still work when a configured-host workflow requires the weights:
the agent must use a license-compliant download/import path, then verify the
model file and hash at the physical SoT.

**Default.** **DO NOT BUNDLE Ultralytics-trained weights.** Provide model loader
infrastructure; acquire weights on the configured host only through
operator-approved/license-compliant local setup. Bundle alternatively licensed
models (CC0 / Apache) when available — e.g., RT-DETR-s with
Apache-2.0-friendly checkpoints.

**Target.** Locked. Track post-v1 alternatives we CAN bundle (license permitting).

---

## OQ-026 — Cross-platform when

**Q.** Linux and macOS land in v2. What's the trigger?

**Trade-off.** Cross-platform doubles surface, expands user base.

**Default.** Start Linux when v1.0 shipped and ≥1000 stars or paying partner interest.

**Target.** Post-v1.

---

## OQ-027 — Operator multi-factor for hardware HID

**Q.** Should hardware HID require a second factor (physical Pico button) instead of CLI flags?

**Trade-off.** Higher friction reduces accidental activation but makes legitimate hardware workflows slower.

**Default.** CLI/config flag + interactive prompt + profile permission metadata is sufficient.

**Target.** v2 if operator-safety feedback calls for it.

---

## OQ-028 — Schema versioning policy

**Q.** Pre-v1 wipe DB on schema change. Post-v1 support migrations or stay wipe-and-rebuild?

**Trade-off.** Migrations: operator-friendly. Wipe: simpler to develop.

**Default.** Wipe at v1.0 (strong release note). Migrations land in v1.1 if real users need.

**Target.** v1.1.

---

## OQ-029 — Notifications channel discipline — DECIDED 2026-05-25

→ decided in ADR-0007.

**Decision.** Synapse delivers notifications per event. The EventBus never
waits to form a batch, and HTTP SSE emits one `synapse/event` frame per
buffered event. Subscribers/clients may batch downstream after receipt, but
that batching must not delay internal producer-to-subscriber delivery. Slow
subscribers use bounded queues/rings with drop-oldest backpressure and explicit
`lossy` state.

---

## OQ-030 — Default GC aggressiveness

**Q.** GC task every 5 min, or wake on cap-exceeded?

**Trade-off.** Periodic: consistent. Reactive: lower idle cost.

**Default.** Periodic 5 min + reactive on writes exceeding soft cap.

**Target.** M5 telemetry. Tune cadence from observed CF growth.

---

## 2. How to use this list

Reader who finds unresolved decision:

1. Check this doc first.
2. If not present, add entry:

```
## OQ-NNN — <one-line summary>

**Q.** ...

**Trade-off.** ...

**Default.** ...

**Target.** <milestone> or <condition>
```

3. Bump next OQ number; don't reuse.

When decided, replace body with:

```
## OQ-NNN — <summary> — DECIDED <date>

→ See ADR-NNN.
```

Parking lot for honest uncertainty; not TODO list.

---

## 3. What this doc does NOT cover

- Resolved decisions (move to ADRs)
- Implementation TODOs (code comments, issue tracker)
- Bug list (issue tracker)
- Feature requests (issue tracker / discussions)
