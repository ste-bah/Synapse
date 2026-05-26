# Synapse Systemspec

Comprehensive technical reference for the Synapse MCP server, produced by reading the source. Every claim in these documents is derived from `crates/` source files cited inline; no aspirational behavior is documented.

## Read order

1. [01_system_overview.md](01_system_overview.md) — architecture map, tech stack, 30-tool inventory, error hierarchy
2. [02_source_code_map.md](02_source_code_map.md) — file tree with per-file descriptions, dep graph, entry-point traces
3. [03_configuration.md](03_configuration.md) — CLI flags, env vars, validation, all numeric defaults
4. [04_storage_layer.md](04_storage_layer.md) — RocksDB schema (11 CFs), schema sentinel, TTL filter, GC, disk pressure
5. [05_core_types_and_errors.md](05_core_types_and_errors.md) — `synapse-core` wire types + 87 error codes
6. [06_mcp_service_and_transports.md](06_mcp_service_and_transports.md) — `SynapseService`, stdio + HTTP routers, Bearer/Origin/Session middleware, SSE bridge
7. [07_reflex_runtime.md](07_reflex_runtime.md) — EventBus, scheduler, the 5 reflex kinds, audit persistence
8. [08_action_subsystem.md](08_action_subsystem.md) — emitter actor, backends, rate limits, hotkey, curves/dynamics
9. [09_perception_and_capture.md](09_perception_and_capture.md) — frame capture, UIA, perception assembler, OCR
10. [10_audio_and_models.md](10_audio_and_models.md) — WASAPI loopback, Whisper-tiny STT, ONNX model loader
11. [11_profiles_hid_telemetry.md](11_profiles_hid_telemetry.md) — TOML profile loader, HID stub, tracing + metrics, test utils
12. [12_milestones_and_roadmap.md](12_milestones_and_roadmap.md) — milestone state, ADRs, doctrine, open decisions
13. [13_mcp_tool_reference.md](13_mcp_tool_reference.md) — every tool's params, defaults, ranges, side effects, errors
14. [14_test_suite.md](14_test_suite.md) — test inventory by crate, run commands, fixtures
15. [15_verification_report.md](15_verification_report.md) — health snapshot, metrics, schema version, constants

## Authority

- `AGENTS.md` and `docs/impplan/00_methodology.md` are the operating doctrine.
  Manual FSV is the shipping gate; this systemspec is descriptive only.
  Missing configured-host prerequisites are acquisition/setup work: agents use
  Synapse/local control as the operator-equivalent host control surface to make
  reversible local prerequisites real, then read the physical source of truth.
  Do not stop at "missing"; if the operator could do it from this computer,
  the agent must do it through Synapse/local host workflows and inspect the SoT.
  Browser downloads, GUI installers, Device Manager checks, package-manager
  installs, model/file generation, firmware flashing, app launching, and UI
  inspection are agent-owned work when reversible on this host.
- For the contract-level PRD, see `docs/computergames/` (numbered 00–17).
- For the per-milestone work-item ledger, see `docs/impplan/` (numbered 00–07).
