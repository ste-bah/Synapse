# Synapse

[![M1 Perception MVP](https://img.shields.io/badge/status-M1_perception_mvp-blue)](https://github.com/ChrisRoyse/Synapse/milestone/2)

Synapse is a Rust MCP server that gives AI agents a local computer-use body: structured perception, action, and low-latency reflexes live in Synapse while the connected model remains the brain. M1 ships the local perception tool surface; action, storage, profiles, and game-control surfaces start in later milestones.

## Agent Doctrine

Agents working in this repository must follow [AGENTS.md](AGENTS.md). Manual
Full State Verification on the configured Windows host is the shipping gate.
Scripts, tests, benchmarks, GitHub Actions, and CI are supporting evidence only;
they are never FSV.

Missing local tools, drivers, models, devices, files, services, account state,
or other prerequisites are acquisition/setup work, not blockers. Agents must
use Synapse/local computer control as an operator-equivalent host control
surface, plus normal OS, shell, browser, package-manager, and device-management
workflows, to make the missing thing real and then read the physical source of
truth directly. Ask only before hard-to-reverse external actions.
Do not stop at "missing": if the operator could do it from this computer,
the agent must use Synapse and local host workflows to make it happen, then
inspect the source of truth.
Browser downloads, GUI installers, Device Manager checks, package-manager
installs, model/file generation, firmware flashing, app launching, and UI
inspection are agent-owned work when they are reversible on this host. A blocker
exists only for the exact hard-to-reverse external action left after that local
work is exhausted.

## Status: M1

M1 is the local perception milestone: a working `synapse-mcp` binary serves MCP over stdio, exposes the six local tools below, and verifies the perception surface through local manual FSV instead of GitHub Actions. The live tracker is the [M1 milestone](https://github.com/ChrisRoyse/Synapse/milestone/2), with mission context pinned in [issue #86](https://github.com/ChrisRoyse/Synapse/issues/86). The implementation checklist is [docs/impplan/02_m1_perception_mvp.md](docs/impplan/02_m1_perception_mvp.md).

## Tools

| Tool | Description | Milestone | Status |
|---|---|---:|---|
| `health` | Reports server version, build, uptime, and subsystem health. | [M0](https://github.com/ChrisRoyse/Synapse/milestone/1) | Done |
| `observe` | Returns the current structured perception snapshot. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `find` | Searches accessible elements and detected entities by role/name/query. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `read_text` | Reads OCR text from a region or element target. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `set_capture_target` | Sets the active primary, monitor, window, or element-window capture target. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `set_perception_mode` | Overrides perception mode between auto, a11y-only, pixel-only, and hybrid. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |

## Build

Use the current installed stable Rust toolchain. M0 is currently verified with Rust 1.95; the repository intentionally does not pin an older toolchain.

```bash
cargo build --release --workspace
```

The release binary is written to:

```text
target/release/synapse-mcp
```

On Windows the binary name is `synapse-mcp.exe`.

## Hardware HID Setup

Hardware HID is optional unless you are exercising the M4 hardware backend. Use
an RP2040 Raspberry Pi Pico board, preferably Pico H (`SC0917`) or Pico WH
(`SC0919`), plus a data-capable micro-USB cable. Buying and source details live
in [docs/hardware/procurement.md](docs/hardware/procurement.md); failure-mode
readbacks live in
[docs/hardware/troubleshooting.md](docs/hardware/troubleshooting.md).

Install the firmware build prerequisites:

```powershell
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs
```

Build the UF2 from the standalone firmware project:

```powershell
cd C:\code\Synapse\firmware\pico-hid
cargo build --release
elf2uf2-rs target\thumbv6m-none-eabi\release\pico-hid pico-hid.uf2
Get-Item .\pico-hid.uf2
```

Hold `BOOTSEL`, plug in the Pico, verify the `RPI-RP2` volume, then copy the
UF2 to that drive:

```powershell
Get-CimInstance Win32_LogicalDisk |
  Where-Object VolumeName -eq 'RPI-RP2' |
  Select-Object DeviceID,VolumeName

Copy-Item .\pico-hid.uf2 E:\
```

Replace `E:` with the actual `DeviceID` from the readback. After the copy,
Windows should dismount `RPI-RP2` and the firmware should re-enumerate. Read
the physical source of truth before using hardware actions:

```powershell
Get-PnpDevice -PresentOnly |
  Where-Object { $_.FriendlyName -match 'Pico|RP2040|Synapse|USB Serial|HID' } |
  Select-Object Status,Class,FriendlyName,InstanceId

Get-CimInstance Win32_SerialPort |
  Select-Object DeviceID,Name,PNPDeviceID
```

Start Synapse with explicit hardware enablement, for example
`synapse-mcp --mode stdio --hardware-hid auto --allow-hardware-hid`.
First-run supported-use acknowledgment is recorded in
`%APPDATA%\synapse\agreement.json`; if it is missing, complete the local
prompt/setup flow and then read that file directly.

## Run

For MCP clients, run stdio mode:

```bash
synapse-mcp --mode stdio
```

Inspect available flags:

```bash
synapse-mcp --help
```

The HTTP transport flag is present for the future surface but returns `NOT_YET_IMPLEMENTED` in the local M1 build.

## Quick Demo

The stdio transport speaks newline-delimited JSON-RPC. A client initializes the server, sends `notifications/initialized`, then calls a tool:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"manual-demo","version":"0.1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"health","arguments":{}}}
```

The health payload shape is:

```json
{
  "ok": true,
  "version": "0.1.0",
  "build": "dev",
  "uptime_s": 0,
  "subsystems": {}
}
```

`uptime_s` is monotonic and `build` is `dev` unless a build SHA is injected.

## Configure MCP Clients

WSL-global install:

```bash
cargo install --path crates/synapse-mcp --force
```

Codex user config at `~/.codex/config.toml`:

```toml
[mcp_servers.synapse]
command = "/home/cabdru/.cargo/bin/synapse-mcp"
args = ["--mode", "stdio"]
```

Claude Code user config:

```bash
claude mcp add --scope user synapse -- /home/cabdru/.cargo/bin/synapse-mcp --mode stdio
```

Claude Desktop on Windows:

```jsonc
// %APPDATA%\\Claude\\claude_desktop_config.json
{
  "mcpServers": {
    "synapse": {
      "command": "C:\\\\Program Files\\\\Synapse\\\\synapse-mcp.exe",
      "args": ["--mode", "stdio"]
    }
  }
}
```

After the client loads the server, ask it to call the Synapse `health` tool and confirm the response has the shape shown above.

## Documentation Map

- Product and architecture PRD: [docs/computergames/README.md](docs/computergames/README.md)
- Implementation plan: [docs/impplan/README.md](docs/impplan/README.md)
- Current Rust/dependency decision: [docs/adr/0001-current-rust-and-dependencies.md](docs/adr/0001-current-rust-and-dependencies.md)

## License

Synapse is licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
