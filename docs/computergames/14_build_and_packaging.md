# 14 — Build and Packaging

## 1. Cargo workspace

`Cargo.toml` at repo root is the workspace manifest declaring members, shared deps, and build profiles.

```toml
[workspace]
resolver = "2"
members = [
    "crates/synapse-mcp",
    "crates/synapse-core",
    "crates/synapse-capture",
    "crates/synapse-a11y",
    "crates/synapse-perception",
    "crates/synapse-audio",
    "crates/synapse-action",
    "crates/synapse-reflex",
    "crates/synapse-storage",
    "crates/synapse-profiles",
    "crates/synapse-hid-host",
    "crates/synapse-models",
    "crates/synapse-telemetry",
    "crates/synapse-test-utils",
    "crates/synapse-overlay",
]
default-members = ["crates/synapse-mcp", "crates/synapse-overlay"]
exclude = ["firmware/pico-hid"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
license = "MIT OR Apache-2.0"
authors = ["Synapse contributors"]
repository = "https://github.com/ChrisRoyse/Synapse"
```

Firmware (`firmware/pico-hid`) is excluded because it targets `thumbv6m-none-eabi`. Separate Cargo project.

---

## 1.1 Configured-host prerequisites

Missing build, firmware, packaging, driver, signing, or release prerequisites
are acquisition/setup work, not blockers by themselves. Agents must use
Synapse/local computer control as the operator-equivalent host control surface,
plus normal OS, shell, browser, package-manager, and device-management
workflows, to download, install, connect, configure, generate, flash, launch,
or inspect the missing prerequisite when those steps are reversible and local to
the configured host. Do not ask the operator to download or install something
while reversible local acquisition/setup remains possible.
Do not stop at "missing." If the operator could do it from this computer, the
agent must use Synapse and host workflows to make it happen, then inspect the
physical source of truth.
Browser downloads, GUI installers, Device Manager checks, package-manager
installs, model/file generation, firmware flashing, app launching, and UI
inspection are agent-owned work when reversible on this host.

The source of truth is the physical place the prerequisite should appear:
package inventory, Rust target list, driver/service list, `Get-PnpDevice`,
registry Enum key, firmware volume, installer path and hash, signing tool
inventory, config file, or equivalent. Ask only before hard-to-reverse external
actions such as spending money, private credentials, billing, external account
changes, or irreversible shared-state changes.

---

## 2. Workspace dependencies

Declared in `[workspace.dependencies]` so every crate uses the same version.
Per ADR-0001, M0 tracks the current compatible crates for the installed stable
Rust toolchain rather than the older planning-time examples.

```toml
[workspace.dependencies]
# Async / IO
tokio = { version = "1.52.3", features = ["full"] }
tokio-util = { version = "0.7.18", features = ["full"] }
crossbeam = "0.8.4"
arc-swap = "1.9.1"

# Serialization
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
toml = "1.1.2"

# Errors / logging
thiserror = "2.0.18"
anyhow = "1.0.102"                    # binary crates only
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "json"] }
tracing-appender = "0.2.5"

# Metrics
metrics = "0.24.6"
metrics-exporter-prometheus = "0.18.3"
opentelemetry = "0.32.0"
opentelemetry-otlp = "0.32.0"

# MCP
rmcp = { version = "1.7.0", features = ["server", "transport-io", "transport-streamable-http-server", "macros", "schemars"] }

# HTTP
axum = "0.8.9"
hyper = "1.9.0"
tower = "0.5.3"

# Windows specific
windows = { version = "0.62.2", features = [
    "Win32_Foundation",
    "Win32_System_Com",
    "Win32_System_Threading",
    "Win32_UI_Accessibility",
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_Graphics_Dxgi",
    "Win32_Graphics_Direct3D11",
    "Media_Ocr",
    "Storage_Streams",
] }
windows-capture = "2.0.0"
uiautomation = { version = "0.25.0", features = ["pattern", "control", "event"] }
chromiumoxide = "0.9.1"

# Audio
wasapi = "0.23.0"

# Input
enigo = "0.6.1"
vigem-client = "0.1.4"
serialport = "4.9.0"

# ML
ort = { version = "2.0.0-rc.12", default-features = false }

# Storage
rocksdb = { version = "0.24.0", default-features = false, features = ["lz4", "zstd", "multi-threaded-cf"] }

# Utility
clap = { version = "4.6.1", features = ["derive"] }
chrono = { version = "0.4.44", features = ["serde"] }
uuid = { version = "1.23.1", features = ["v4", "v7", "serde"] }
schemars = { version = "1.2.1", features = ["derive"] }
regex = "1.12.3"
sha2 = "0.11.0"
crc16 = "0.4"
notify = "9.0.0-rc.4"

# Dev / test
proptest = "1.11.0"
criterion = "0.8.2"
insta = "1.47.2"
tempfile = "3.27.0"
mockall = "0.14.0"

[workspace.lints.rust]
unsafe_code = "forbid"                # overridden per-crate where needed
unused = "warn"

[workspace.lints.clippy]
all = "deny"
pedantic = "warn"
nursery = "warn"
unwrap_used = "deny"                  # forbid unwrap() outside tests
expect_used = "deny"
```

Per-crate `Cargo.toml` adds `unsafe_code = "allow"` only where needed (`synapse-capture`, `synapse-hid-host` for serial OS handles).

---

## 3. Build profiles

```toml
[profile.dev]
opt-level = 0
debug = "line-tables-only"
incremental = true
lto = false

[profile.release]
opt-level = 3
debug = "limited"
incremental = false
lto = "thin"
codegen-units = 16
strip = true
panic = "abort"

[profile.release-max]
inherits = "release"
codegen-units = 1
lto = "fat"

[profile.bench]
inherits = "release"
debug = "line-tables-only"
```

`release` ships and strips symbols for the M0 binary-size gate. `release-max` is for benchmarks-of-record and absolute fastest binaries. `bench` keeps line-tables for `criterion` flamegraphs.

`panic = "abort"` because Synapse's `release_all` runs through the panic hook anyway; unwinding adds binary size without benefit.

---

## 4. Build commands

```powershell
# Standard build
cargo build --release

# Build all examples (supporting regression artifacts)
cargo build --release --examples

# Build only the binary
cargo build --release -p synapse-mcp

# Run tests
cargo test --workspace

# Build firmware (separate)
cd firmware/pico-hid
cargo build --release --target thumbv6m-none-eabi
elf2uf2-rs target/thumbv6m-none-eabi/release/synapse-pico-hid synapse-pico-hid.uf2
```

Local supporting checks follow the matrix in `13_testing_strategy.md` §14.

---

## 5. Feature flags

| Flag | Default | Effect |
|---|---|---|
| `rocksdb-backend` | on | Use RocksDB for storage (default and only M3 backend) |
| `cuda` | off | ORT with CUDA execution provider |
| `directml` | on | ORT with DirectML (default GPU path on Windows) |
| `vlm` | off | Bundle a small VLM for `describe` |
| `perf-profiling` | off | Compile with `tracing-flame` + `pprof` |
| `overlay` | on | Build the debug overlay subbinary |

Default ship build: `rocksdb-backend + directml + overlay`. Operators wanting CUDA pass `--features cuda` at install.

---

## 6. Installation

### 6.1 Via cargo

```powershell
cargo install --git https://github.com/ChrisRoyse/Synapse synapse-mcp --features directml
cargo install --git https://github.com/ChrisRoyse/Synapse synapse-overlay --features overlay
```

For users with the Rust toolchain. Builds from source.

### 6.2 Via prebuilt installer (Windows MSI)

For users without Rust. Signed Windows MSI from `wix-installer`:

- `synapse-mcp.exe` installed to `C:\Program Files\Synapse\`
- `synapse-overlay.exe` alongside
- Start menu shortcuts
- Bundled ONNX models for default detection + OCR + STT
- Bundled profiles
- Bundled RP2040 firmware `.uf2`
- Visual C++ runtime redistributable (RocksDB dep)
- Optional checkbox: launch the Nefarius signed ViGEmBus installer for an
  operator-completed GUI install

MSI signed with project's code-signing cert (community-trusted; operator sees "Verified Publisher" SmartScreen prompt first time).

### 6.3 Via winget

```powershell
winget install Synapse.SynapseMCP
```

Published to community winget repo after v1.0. Refers to the signed MSI.

### 6.4 Via chocolatey

Optional, community-maintained. Same MSI.

### 6.5 Portable zip

Air-gapped installs: `.zip` with binaries + bundled models + profiles. Extract anywhere; run from there. No installer machinery.

---

## 7. First-run setup

`synapse-mcp setup` is the one-time wizard:

1. **Permissions check.** Confirm user can write to `%LOCALAPPDATA%\synapse\`.
2. **Supported-use acknowledgment.** (Per `08` §5.)
3. **ViGEmBus check.** Detect; if missing, offer to download and launch the
   Nefarius installer for operator GUI clickthrough.
4. **Model selection.** Show detection / OCR / STT models with sizes. Operator picks which to download.
5. **Profile selection.** Show profiles; default = enable all bundled.
6. **Bearer token generation.** For HTTP mode; store in `%APPDATA%\synapse\token.txt`.
7. **Optional hardware HID.** Detect connected RP2040 boards; offer to flash one.
8. **Start configuration.** Write `%APPDATA%\synapse\config.toml`.
9. **First server start.** Launches `synapse-mcp --mode stdio` and prompts operator to configure their agent client.

Supports `--non-interactive --accept-defaults` for headless installs except
for ViGEmBus provisioning on current Win11. The Nefarius 1.22.0 installer is
known to fail with silent/extraction flags, so non-interactive setup must skip
ViGEmBus and report that gamepad support requires GUI installation. Under the
configured-host doctrine, the agent should use normal Windows/browser/installer
workflows and Synapse computer control to complete that GUI setup when gamepad
support is required, then read the real driver/service source of truth. For M2,
the release gate is manual FSV on the configured operator host where ViGEmBus is
installed and verified working.

---

## 8. config.toml schema

```toml
# %APPDATA%\synapse\config.toml

[server]
default_mode = "stdio"               # stdio | http
http_bind = "127.0.0.1:7700"
allow_non_loopback = false
tls_cert = ""
tls_key = ""

[storage]
db_path = ""                         # default: %LOCALAPPDATA%\synapse\db
nightly_compaction_hour = 3          # UTC hour
profiles_dir = ""                    # default: %APPDATA%\synapse\profiles

[retention]
# Per-CF overrides; see 07_storage_and_profiles.md §4

[logging]
level = "info"
log_dir = ""                          # default: %LOCALAPPDATA%\synapse\logs
otlp_endpoint = ""                    # empty = disabled

[metrics]
prometheus_bind = ""                  # empty = disabled, e.g., "127.0.0.1:9100"

[capture]
default_target = "primary"
min_update_interval_ms = 16

[perception]
default_mode = "auto"

[detection]
default_model = "yolov10n_general"
backend_preference = ["cuda", "directml", "cpu"]

[ocr]
default_backend = "winrt"

[audio]
loopback_enabled = true
stt_model = "whisper-tiny-int8"

[action]
default_keyboard_backend = "software"
default_mouse_backend = "software"
default_pad_backend = "vigem"
hardware_hid_port = ""                # empty = auto-detect

[safety]
panic_hotkey = "ctrl+alt+shift+p"
allow_launch = []                     # list of regexes
allow_shell = []                      # list of regexes
allow_hardware_hid = false
no_redaction = false
require_acknowledge_on_start = true

[redaction]
[redaction.custom_patterns]
# operator-extensible
```

Schema versioned via `synapse_config_version = "1"` at top. Mismatch refuses to start with `CONFIG_VERSION_MISMATCH`.

---

## 9. CLI surface

`synapse-mcp` is the daemon entry plus sub-commands:

```
synapse-mcp [SUBCOMMAND]

Commands:
  (none)                Run as MCP server (default)
  setup                 Run first-time setup wizard
  health                Print health check and exit
  db status             Print DB summary
  db gc [--aggressive]  Run garbage collection
  db compact            Force compaction
  db wipe [--yes]       Wipe the database
  db backup <out>       Hot backup to a directory
  db restore <in>       Restore from a backup directory
  db trim --cf <name> --keep-hours <n>  Manual trim
  models list           List cached / available models
  models import <path>  Side-load a model
  models gc             Drop unreferenced models
  profiles list         List loaded profiles
  profiles install <src>  Install a profile from path or URL
  profiles validate <path>  Validate a profile file
  replay list           List replay sessions
  replay show <id>      Show session summary
  replay export <id> <out.zip>
  replay tail <id>
  metrics dump --since <duration> --output <path>
  hid identify --port <com>
  hid flash --port <com>
  token rotate
  overlay               Launch debug overlay (separate process)
  --version             Print version + build hash + signature
  --help

Top-level flags:
  --mode <stdio|http>
  --bind <addr>
  --db <path>
  --profile-dir <path>
  --log-level <level>
  --reflex-disabled
  --vigem-disabled
  --hardware-hid <port|auto>
  --allow-launch <regex>
  --allow-shell <regex>
  --allow-hardware-hid
  --no-redaction
  --otlp-endpoint <url>
  --metrics-bind <addr>
  --no-tray
```

All flags map to `SYNAPSE_*` env vars (e.g., `SYNAPSE_MODE=http`).

---

## 10. Logo, icons, branding

- `assets/logo.svg` — vector logo
- `assets/icon-256.png`, `assets/icon-32.png`, `assets/icon-16.png` — Windows icons
- `assets/installer-banner.png` — MSI installer header
- `assets/tray-icon-active.ico`, `assets/tray-icon-paused.ico`, `assets/tray-icon-error.ico`

Simple design (circle with fork-like symbol). Replaceable; no strong brand requirement.

---

## 11. Code signing

We sign:

- `synapse-mcp.exe` (daemon)
- `synapse-overlay.exe`
- `SynapseSetup-x.y.z.msi` (installer)
- `synapse-pico-hid.uf2` (firmware; signature embedded in metadata payload — informational, not cryptographic)

Cert: EV code-signing held by the maintainer (post-v1, when funded). Pre-v1: self-signed; operators see SmartScreen warning until trust builds.

Sign with `signtool.exe` from the Windows SDK:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a synapse-mcp.exe
```

Part of `scripts/release/sign.ps1`.

---

## 12. Release process

1. **Branch:** `release/x.y.z` cut from `main`.
2. **Tag:** `vx.y.z` on the commit.
3. **Local release build/signing step** builds:
   - `synapse-mcp.exe` (release profile, signed)
   - `synapse-overlay.exe` (signed)
   - `SynapseSetup-x.y.z.msi` (signed)
   - `synapse-portable-x.y.z-windows-x64.zip`
   - `synapse-pico-hid-x.y.z.uf2`
4. **Upload to GitHub Releases** with release notes.
5. **Publish to crates.io** for `synapse-mcp` (cargo-installable).
6. **Update winget manifest** PR.

Maintainer signs off per manual test plan in `13_testing_strategy.md` §15.

---

## 13. Reproducible builds

Goal: a given commit hash produces byte-identical binaries on any contributor's machine.

- Use the installed stable Rust toolchain recorded in `docs/adr/0001-current-rust-and-dependencies.md`
- `cargo build --frozen --locked`
- Pin all deps in `Cargo.lock` (committed)
- Avoid build.rs that touches network or system clock
- ONNX models referenced by sha; never bundled in the binary (downloaded at install or first run)

Bitwise-reproducible Windows binaries (PE timestamps, COFF section ordering) not yet pursued. Post-v1.

---

## 14. License compliance

`cargo deny check` enforces:

- Only `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `MPL-2.0`, `ISC`, `Zlib`, `Unicode-3.0`, `BSL-1.0`, `CC0-1.0` allowed at M0
- `GPL-*`, `AGPL-*`, `SSPL-*` blocked
- Vendored deps without SPDX identifier blocked

`THIRD-PARTY-LICENSES.md` generated by `cargo about`, included in installer.

---

## 15. Bundled models and binary size

Default install size targets:

| Component | Size |
|---|---|
| `synapse-mcp.exe` (stripped, LTO) | ≤ 15 MB |
| `synapse-overlay.exe` | ≤ 10 MB |
| ONNX models bundled (YOLOv10n, Whisper-tiny; WinRT-OCR is OS-provided) | ≤ 80 MB |
| Profiles | ≤ 1 MB |
| Total MSI | ≤ 120 MB |

`describe` VLM is NOT bundled; downloaded on first use (~500 MB). Operator can opt out and skip `describe`.

---

## 16. Update mechanism

Synapse does not auto-update. Updates via:

- `winget upgrade Synapse.SynapseMCP`
- Downloading a new MSI manually
- `cargo install --git ... --force` for cargo installs

At startup, Synapse optionally checks GitHub Releases (`--check-updates` opt-in; off by default) for new versions and prints a one-line notice. No data sent except User-Agent.

---

## 17. Crate-by-crate Cargo.toml templates

Each crate's `Cargo.toml`:

```toml
[package]
name = "synapse-foo"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
synapse-core = { path = "../synapse-core" }
# ... per-crate deps from [workspace.dependencies]

[dev-dependencies]
synapse-test-utils = { path = "../synapse-test-utils" }
proptest.workspace = true
insta.workspace = true
tempfile.workspace = true

[lints]
workspace = true
```

Templates in `scripts/new-crate.ps1`:

```powershell
.\scripts\new-crate.ps1 -Name synapse-new
```

Generates a skeleton crate following the template.

---

## 18. Documentation generation

`cargo doc --workspace --no-deps` generates API docs. Published to docs.rs for library crates (`synapse-core`, `synapse-storage`, etc.). The binary crate (`synapse-mcp`) is not on docs.rs; `--help` and this PRD are canonical.

---

## 19. What this doc does NOT cover

- Supporting automation configuration → `.github/workflows/`
- Distribution channel publishing details → `scripts/release/`
- Firmware build details → `09_hardware_hid_gateway.md` §8
- Per-feature-flag testing combinations → `13_testing_strategy.md` §14
