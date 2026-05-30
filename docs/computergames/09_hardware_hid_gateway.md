# 09 — Hardware HID Gateway

## 1. Why hardware

`SendInput`, `keybd_event`, `mouse_event`, and ViGEm virtual controllers are software-layer input. Some accessibility, research, and simulation setups need output that the OS receives as a physical peripheral. We want a real-device path for three reasons:

1. **Accessibility.** A user with motor impairments using eye-tracking or sip-and-puff input deserves a peripheral the OS treats the same as a real mouse. Synapse becoming that bridge is valuable.
2. **AI research and tournaments.** Sanctioned AI tournaments and university research often require the AI's output to flow through real hardware for fair comparison with human play.
3. **Demo recording / sim rigs.** People building dedicated rigs (sim cockpits, arcade cabinets, modded controllers) want a programmable HID device.

Optional component; Synapse runs without it. Build and flash a board only when the workflow needs a physical-input path.

This doc specifies firmware design, host-side serial driver in `synapse-hid-host`, and wire protocol.

---

## 2. Hardware choices

M4's reference platform is the RP2040 Raspberry Pi Pico family. All M4 firmware
is Rust, embedded async via `embassy`, and targets `thumbv6m-none-eabi`.

| Board | Cost | Why |
|---|---|---|
| **Raspberry Pi Pico / Pico H / Pico WH (RP2040)** | ~$4-6 | M4 default. Cheap. Easy to source. Stable USB stack via `embassy-usb`. Pico H/WH avoid soldering headers for future lab work. |
| **Raspberry Pi Pico 2 / Pico 2 H (RP2350)** | ~$5-7 | Out of M4. Requires an explicit RP2350 port/feature flag before it can be a supported first board. |
| **Arduino Pro Micro / Leonardo (ATmega32u4)** | ~$10 | Out of M4. Would be a later stripped firmware port, not the Synapse M4 acceptance path. |

Default and primary: **Raspberry Pi Pico (RP2040)**. Rest of doc assumes RP2040.

### Bill of materials (minimum viable)

- 1x Raspberry Pi Pico / Pico H / Pico WH (RP2040)
- 1x data-capable micro-USB cable to the host PC
- Optional: small project box

No external components. Power and data over the same USB.

Hardware sourcing and BOOTSEL setup details live in
[`../hardware/procurement.md`](../hardware/procurement.md).

If any M4 hardware, cable, driver, firmware artifact, build tool, serial port,
or host surface is missing, the agent must make the real prerequisite happen on
this configured host when reversible local steps exist. Do not stop at
"missing"; Synapse gives the agent full local computer-control responsibility,
equivalent to the operator's local control for reversible host work, so use
Synapse and local OS/browser/package/device workflows, then read the physical
source of truth (`RPI-RP2` volume, `Get-PnpDevice`, COM port,
driver/service inventory, artifact hash, or equivalent).

---

## 3. Device identity

Board enumerates as a **USB HID composite device** with three interfaces:

| Interface | Class | Subclass | Protocol | What it is |
|---|---|---|---|---|
| 0 | HID (3) | Boot (1) | Mouse (2) | Boot-protocol mouse — works in BIOS, Windows native |
| 1 | HID (3) | Boot (1) | Keyboard (1) | Boot-protocol keyboard |
| 2 | HID (3) | None (0) | None (0) | Standard HID gamepad (DirectInput-visible, Xbox-like 14-byte report; ADR-0009) |

Plus a CDC ACM command channel, which enumerates as a control/data interface
pair:

| Interface | Class | Purpose |
|---|---|---|
| 3 | CDC Communications | ACM control interface for the Synapse serial command channel |
| 4 | CDC Data | Serial data interface from Synapse host driver to firmware |

VID/PID defaults:

```
VID: 0x2E8A  (Raspberry Pi RP2040 VID)
PID: 0x1F50  (Synapse Pico HID internal reservation; ADR-0008)
Manufacturer: Synapse
Product: Synapse Pico HID
```

The canonical constants live in `synapse-core::usb_identity` and are imported
by `firmware/pico-hid/src/usb.rs`. Operators can rebuild firmware with their own
`VID`/`PID`/`MANUFACTURER_STR`/`PRODUCT_STR`, but release firmware ships only the
ADR-0008 Synapse identity.

Before broad public distribution, Synapse should complete the Raspberry Pi
`usb-pid` application/PR for the chosen PID or update ADR-0008 if Raspberry Pi
assigns a replacement.

Source notes: USB identity is locked by
[`../adr/0008-usb-vid-pid-pico-hid.md`](../adr/0008-usb-vid-pid-pico-hid.md);
the gamepad-vs-XInput decision is locked by
[`../adr/0009-hid-gamepad-vs-xinput.md`](../adr/0009-hid-gamepad-vs-xinput.md).

---

## 4. Firmware architecture (RP2040, Rust, embassy)

```
firmware/pico-hid/
├── Cargo.toml
├── memory.x                    # RP2040 linker
├── build.rs                    # copies memory.x into OUT_DIR and passes linker args
├── src/
│   ├── main.rs                 # entry point, embassy executor, LED heartbeat
│   ├── usb.rs                  # imports canonical Synapse USB identity
│   ├── hid_descriptors.rs      # report descriptors (mouse, kbd, pad)
│   ├── reports.rs              # boot mouse, boot keyboard, 14-byte gamepad structs
│   ├── serial.rs               # USB composite builder, CDC ACM parser, HID writers
│   ├── protocol.rs             # frame parser/encoder and command constants
│   ├── dispatch.rs             # command dispatcher, identify, telemetry, report state
│   ├── safety.rs               # watchdog helper and release_all-on-timeout behavior
│   └── led.rs                  # status LED feedback
└── tests/
    ├── led_patterns.rs
    ├── loopback_dispatch.rs
    ├── protocol_roundtrip.rs
    └── safety_watchdog.rs
```

UF2 conversion is performed after `cargo build` with `elf2uf2-rs`; `build.rs`
does not create the UF2 by itself.

### 4.1 Embassy executor

```rust
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    let usb_ready = serial::spawn_usb(peripherals.USB, &spawner);
    let mut led = Output::new(peripherals.PIN_25, Level::Low);

    loop {
        // LED state is derived from USB task startup and later command/watchdog inputs.
        let output = led_output(/* LedInputs */);
        if output.on { led.set_high(); } else { led.set_low(); }
        Timer::after_millis(LED_TICK_MS).await;
    }
}
```

Implementation checkpoint: current `main.rs` calls `serial::spawn_usb(...)`,
then runs the LED loop. `serial.rs` builds the USB device, CDC ACM serial class,
and three HID writers, parses CDC frames, dispatches through `dispatch.rs`, and
feeds shared runtime report state into the live HID writers. `RuntimeState`
polls `safety::Watchdog`, tracks parser/drop telemetry, and feeds LED inputs.

### 4.2 Cooperative loops

| Task | Purpose | Latency target |
|---|---|---|
| `device_task` | USB stack background pump | n/a; embassy-driven |
| `serial_task` | Reads framed bytes from CDC, parses commands, dispatches | ≤ 0.5 ms per command |
| `command_dispatcher` | Applies command to relevant HID interface | ≤ 1 ms host→USB-on-wire |
| `watchdog helper` | `safety::Watchdog` releases all inputs when polled after timeout; issue #375 wires polling into the hardware loop | resolution 50 ms target |
| `led_indicator` | Blinks status (idle / active / error) | n/a |

### 4.3 HID descriptors

**Mouse (boot protocol).** Standard boot mouse with 3 buttons, relative X/Y
8-bit deltas, and vertical wheel. Host-side absolute/large relative movement is
chunked into the firmware's `-127..=127` relative range before transmission.

**Keyboard (boot-protocol superset).** 8-byte boot keyboard report: modifiers byte + reserved + 6 keycodes. Reports HID Usage IDs directly.

**Gamepad.** Standard HID gamepad report, not XInput/XUSB emulation (ADR-0009):

```
buttons: u16,        // standard Button page bitfield: A,B,X,Y, LB,RB, Back,Start, LS,RS, DUp,DDown,DLeft,DRight, Guide, Reserved
left_trigger: u8,
right_trigger: u8,
thumb_lx: i16,
thumb_ly: i16,
thumb_rx: i16,
thumb_ry: i16,
reserved: u16,      // zero; keeps the M4 14-byte report ABI explicit
```

Total 14 bytes. Sent at up to 1000 Hz. Games that require XInput/XUSB should
use the ViGEm backend; the hardware pad is a real HID/DirectInput-visible
peripheral.

---

## 5. Wire protocol (host ↔ firmware)

Host (`synapse-hid-host` crate) talks to firmware over USB CDC ACM at **1 Mbaud** (informational; CDC ACM isn't baud-rate-limited, but most host drivers respect the setting for buffering).

Binary, framed, with explicit acks. Parseable with no allocations on firmware side.

### 5.1 Frame layout

```
+--------+-------+--------+----------+-------+-----+
| MAGIC  | LEN   | SEQ    | CMD      | PAYLOAD| CRC|
| 0x5A   | u16le | u32le  | u8       | bytes  | u16le|
+--------+-------+--------+----------+-------+-----+
```

- `MAGIC`: 0x5A (sync; firmware resyncs by skipping bytes until it sees this)
- `LEN`: total frame length excluding `MAGIC`, including `CRC`
- `SEQ`: monotonic sequence number assigned by host
- `CMD`: command identifier
- `PAYLOAD`: command-specific
- `CRC`: CRC16/CCITT-FALSE over `LEN..CRC`

### 5.2 Commands (host → firmware)

| `CMD` | Name | Payload | Effect |
|---|---|---|---|
| 0x01 | `PING` | `[u32 nonce]` | firmware echoes `PONG` with same nonce |
| 0x02 | `IDENTIFY` | empty | firmware replies with `IDENTIFY_RESP { fw_major, fw_minor, fw_patch, build_hash, vid, pid, capabilities_mask }` |
| 0x10 | `MOUSE_MOVE_REL` | `[i16 dx][i16 dy]` | mouse delta |
| 0x11 | `MOUSE_BUTTON` | `[u8 button][u8 down_flag]` | button state |
| 0x12 | `MOUSE_WHEEL` | `[i8 dy][i8 dx]` | wheel ticks |
| 0x20 | `KEY_DOWN` | `[u8 hid_code]` | keyboard key down |
| 0x21 | `KEY_UP` | `[u8 hid_code]` | keyboard key up |
| 0x22 | `KEY_MODS` | `[u8 mods_bitfield]` | set modifier state directly |
| 0x30 | `PAD_REPORT` | `[14 bytes raw report]` | apply pad report |
| 0x40 | `RELEASE_ALL` | empty | all mouse buttons up, all keys up, pad neutral |
| 0x50 | `WATCHDOG_KICK` | `[u32 timeout_ms]` | reset watchdog with new timeout |
| 0x60 | `GET_TELEMETRY` | empty | replies with `TELEMETRY_RESP` counters and command-timing telemetry |
| 0xF0 | `RESET_TO_BOOTLOADER` | empty | enters UF2 bootloader (for re-flashing) |

### 5.3 Responses (firmware → host)

Same frame layout, `MAGIC = 0xA5` (mirror byte) to distinguish direction.

| `CMD` | Name | Payload |
|---|---|---|
| 0x80 | `ACK` | `[u32 seq_acked]` |
| 0x81 | `NAK` | `[u32 seq_acked][u8 reason_code]` |
| 0x82 | `PONG` | `[u32 nonce]` |
| 0x83 | `IDENTIFY_RESP` | (see above) |
| 0x84 | `TELEMETRY_RESP` | (see above) |
| 0x90 | `EVENT_BUTTON_PRESS_LOCAL` | (reserved; future: physical buttons on the board) |

Source note: host and firmware constants are mirrored in
`firmware/pico-hid/src/protocol.rs` and
`crates/synapse-hid-host/src/protocol.rs`.

`TELEMETRY_RESP` currently carries an 11-field little-endian `u32` payload
(44 bytes):

1. `uptime_ms`
2. `frames_received`
3. `frames_dropped`
4. `link_errors`
5. `commands_executed`
6. `watchdog_fires`
7. `crc_errors`
8. `timed_commands`
9. `previous_command_delta_us`
10. `last_command_delta_us`
11. `last_timed_command_uptime_us`

The host parser also accepts the legacy 7-field base payload
(`TELEMETRY_BASE_PAYLOAD_LEN = 28`) so old firmware can still be identified
without corrupting the timing fields; the four timing values are represented as
`None` when absent. The current firmware writer returns the full
`TELEMETRY_PAYLOAD_LEN = 44` payload.

### 5.4 Sequence numbers and ack semantics

Host assigns monotonic `SEQ`. Firmware replies with `ACK` for each accepted
frame and `NAK` for parser/dispatcher rejection. The current host pipeline
waits 5 ms for an ACK/NAK and retries the same `SEQ` up to 3 times; after the
retry budget is exhausted, host raises `HID_LINK_TIMEOUT` and surfaces
`ACTION_HID_PORT_DISCONNECTED` to caller.

For volume input (e.g., a curve emitting 50 small mouse moves), host writes up
to 16 outstanding unacked frames through `HidPipeline`. Firmware currently reads
CDC packets into a `MAX_FRAME_LEN` receive buffer, parses complete frames
synchronously, and dispatches each parsed frame immediately. `NAK_BUFFER_FULL`
is currently emitted when command state cannot accept more data, such as the
6-key boot-keyboard rollover slots already being full.
The real-device single-retry acceptance image is built with
`.\scripts\release\firmware\build_pico_hid.ps1 -Features force-first-nak`; it
leaves `IDENTIFY` untouched, emits one `NAK_BUFFER_FULL` for the first normal
ACK-style command, then accepts the retry with the same sequence.

ADR-0012 reduces unnecessary hardware mouse traffic before this pipeline sees
curve-generated batches. After action curve sampling and `-127..=127`
boot-mouse chunking, the hardware backend coalesces adjacent same-direction
`MOUSE_MOVE_REL` deltas when their implied span is `<= 2 ms` and the merged
payload remains inside the firmware range. Software actions and standalone
direct `MouseMoveRelative` calls are unchanged.

### 5.5 NAK reason codes

```
0x01 NAK_CRC_INVALID
0x02 NAK_LEN_INVALID
0x03 NAK_UNKNOWN_CMD
0x04 NAK_PAYLOAD_INVALID
0x05 NAK_BUFFER_FULL
0x06 NAK_WATCHDOG_EXPIRED       // defined for watchdog refusal; issue #375 wires emission into the hardware loop
```

### 5.6 Frame loss handling

USB CDC ACM is reliable in practice. CRC + ack detects protocol bugs and link-level glitches (cable disconnect, unplug-replug). Frame loss is not expected during normal operation.

---

## 6. Safety: the watchdog

M4 watchdog contract: if no command is received within
`WATCHDOG_TIMEOUT_MS`/`DEFAULT_WATCHDOG_TIMEOUT_MS` (default 1000 ms), firmware
must:

1. Log event internally (telemetry counter increments)
2. Issue internal `RELEASE_ALL` — all mouse buttons up, all keys up, gamepad neutral
3. Continue running, ready for new commands

Prevents stuck inputs if the host process crashes or USB link freezes mid-action.

Current implementation note: `firmware/pico-hid/src/safety.rs` implements
`Watchdog::poll`, `DEFAULT_WATCHDOG_TIMEOUT_MS = 1000`, and
`WATCHDOG_DISABLED_TIMEOUT_MS = 0`. `Watchdog::poll` calls
`DispatchState::release_all()` and records watchdog telemetry when it fires.
Host timeout tuning is carried by `WATCHDOG_KICK`; disabling by setting timeout
to `0` is defined but not recommended because it removes stuck-input safety.
Issue #375 closes the remaining hardware-loop polling and manual telemetry
evidence.

---

## 7. Host-side driver (`synapse-hid-host`)

```rust
pub struct HidGateway {
    port_name: String,
    baud_rate: u32,
    read_timeout: Duration,
    identity: FirmwareIdentity,
    pipeline: HidPipeline,
    port: Box<dyn SerialPort>,
}

impl HidGateway {
    pub fn connect(port_name: impl Into<String>) -> HidResult<Self> {
        // Verifies the port is present, opens serialport at 1_000_000 baud
        // with a 5 ms read timeout, then runs IDENTIFY.
    }

    pub fn send_command(&mut self, command: u8, payload: &[u8]) -> HidResult<u32> {
        self.pipeline.send_command(self.port.as_mut(), command, payload)
    }

    pub fn send_commands(&mut self, commands: &[HostCommandRequest<'_>])
        -> HidResult<Vec<u32>>
    {
        self.pipeline.send_commands(self.port.as_mut(), commands)
    }

    pub fn get_telemetry(&mut self) -> HidResult<HidTelemetrySnapshot> {
        self.pipeline.get_telemetry(self.port.as_mut())
    }
}
```

The current host driver is synchronous around `serialport`. `HidPipeline`
provides the bounded sliding window, ACK/NAK parsing, retry budget, and
`GET_TELEMETRY` request/`TELEMETRY_RESP` parsing. `ReconnectGateway` wraps a
connected link in a worker thread for reconnect attempts and exposes the same
telemetry snapshot read through the active link.

`HidTelemetrySnapshot` stores the seven base counters plus optional timing
fields (`timed_commands`, `previous_command_delta_us`,
`last_command_delta_us`, `last_timed_command_uptime_us`) so host benches can
distinguish firmware that lacks the timing extension from firmware that reports
zero timing values.

### 7.1 Auto-detect

`synapse-mcp` at startup with `--hardware-hid auto` enumerates COM ports,
filters USB serial candidates by the Synapse VID/PID from ADR-0008, sends
`IDENTIFY` through `HidGateway::connect`, and uses the first candidate whose
identity handshake succeeds. Error if none succeeds.

### 7.2 Reconnection

On serial error (port closed, USB unplugged), `ReconnectGateway` enters a
disconnected/connecting state and retries every 500 ms. While disconnected, all
action calls using `Backend::Hardware` return `ACTION_HID_PORT_DISCONNECTED`
immediately (no queueing).

### 7.3 Firmware version handshake

`IDENTIFY_RESP` is 20 bytes: `[fw_major u8][fw_minor u8][fw_patch u8][reserved u8][build_hash 8 bytes][vid u16le][pid u16le][capabilities u32le]`. Host compares `fw_major` against compiled-in `EXPECTED_FW_MAJOR`. Mismatch returns `HID_FIRMWARE_VERSION_MISMATCH` and aborts. Operator runs `synapse-mcp hid flash` to update. The real-device mismatch image is built with `.\scripts\release\firmware\build_pico_hid.ps1 -Features fake-fw-major-mismatch`, which emits `pico-hid-fake-fw-major-mismatch-<version>.uf2` and changes only the advertised identify major.

---

## 8. Building and flashing the firmware

```powershell
# One-time
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs

# Build versioned release artifacts
.\scripts\release\firmware\build_pico_hid.ps1 -Version 0.1.0
.\scripts\release\firmware\build_pico_hid.ps1 -Version 0.1.0-m4
.\scripts\release\firmware\build_pico_hid.ps1 -Version 0.1.0 -Features loopback

# Flash
# 1. Hold BOOTSEL on the Pico while plugging USB
# 2. Pico appears as a USB mass storage device "RPI-RP2"
# 3. Copy the selected versioned UF2 to it; Pico reboots into Synapse firmware
```

The helper writes artifacts under `scripts\release\firmware\` using the naming
pattern `pico-hid-<version>.uf2` for the default firmware and
`pico-hid-<feature-list>-<version>.uf2` for feature builds such as
`pico-hid-loopback-0.1.0.uf2`.

If any build, flash, driver, cable, board, or firmware artifact prerequisite is
missing, the agent must make the real prerequisite exist on this configured host
before declaring the issue blocked: install the tool, acquire or connect the
device when locally reversible, put the board in BOOTSEL, inspect Device Manager
or `Get-PnpDevice`, copy the UF2, inspect USB/COM state, and then read the
physical source of truth that proves the prerequisite or flashed firmware is
present. Ask the operator only for hard-to-reverse external actions such as
spending money or using private credentials.

Helper: `synapse-mcp hid flash --port COM7`:

1. Detect if device is in Synapse firmware mode (sends `IDENTIFY`).
2. If yes, send `RESET_TO_BOOTLOADER` to reboot into UF2.
3. Wait for mass storage to appear.
4. Copy bundled/versioned `pico-hid-0.1.0-m4.uf2` unless another explicit UF2
   is selected.
5. Wait for re-enumeration as Synapse firmware.
6. Verify with `IDENTIFY`.

Bundled `.uf2` files are released as GitHub release assets per Synapse version, signed by the project key.

---

## 9. Power and electrical

- USB bus-powered. ~50 mA under load. Pico's regulator handles 5 V input fine.
- No external components for reference design.
- Optional: tactile button on GP0 as emergency unplug (firmware reads; on press, sends `RELEASE_ALL` and clears state).

Status LED:

| LED state | Meaning |
|---|---|
| Slow heartbeat (0.5 Hz) | Idle / no commands for at least 5 s |
| Steady on | Receiving commands actively |
| Fast blink (5 Hz) | Watchdog fired (released all) |
| SOS pattern | Firmware error; reflash needed |

---

## 10. Performance budget

| Stage | Target p99 |
|---|---|
| Host: action → serial bytes on wire | ≤ 200 µs |
| USB CDC bus latency (full-speed USB) | ~1 ms (USB poll interval) |
| Firmware: parse frame → HID report ready | ≤ 100 µs |
| Firmware: HID report → on the USB IN endpoint | next 1 ms poll |
| End-to-end: host call → physical USB IN packet | ≤ 4 ms p99 |

1 ms USB poll is the hard floor. Hardware HID will always be ~3 ms slower than software `SendInput` (which doesn't go over USB). That is the cost of going through a physical USB device.

Hardware curve coalescing keeps sub-2 ms same-direction cursor detail from
creating more CDC frames than the nominal HID poll floor can expose, while the
firmware range check still prevents overlarge mouse payloads.

---

## 11. Testing the firmware

| Test | How |
|---|---|
| Protocol roundtrip | `cd firmware/pico-hid; cargo test --tests` (host-side parser tests with hand-crafted frames) |
| Firmware loopback | Build with `.\scripts\release\firmware\build_pico_hid.ps1 -Features loopback`; firmware echoes every command back as `PONG`. Host driver sends 1000 commands, asserts all return. |
| Firmware major mismatch | Build with `.\scripts\release\firmware\build_pico_hid.ps1 -Features fake-fw-major-mismatch`; host `IDENTIFY` must fail with `HID_FIRMWARE_VERSION_MISMATCH` after flashing the image to a real Pico. |
| Firmware forced retry | Build with `.\scripts\release\firmware\build_pico_hid.ps1 -Features force-first-nak`; first ACK-style command must return one `NAK_BUFFER_FULL`, retry the same sequence, then ACK. |
| Watchdog | Connect, send commands, stop >1s, observe `RELEASE_ALL` via internal telemetry. |
| Stress | Send 10,000 mouse-move-rel commands at full rate; assert no drops, all acked. |
| Re-enumeration | Trigger `RESET_TO_BOOTLOADER`, observe device drops, mass storage appears, reflash, reconnect. |

Local protocol roundtrip checks run without hardware. Firmware-loopback,
watchdog, stress, and re-enumeration rows require the configured host and real
hardware when they are used as supporting evidence. They are not FSV by
themselves; manual FSV still triggers the real runtime surface and separately
reads the physical source of truth after each action.

---

## 12. Limitations and notes

- **Full-speed USB only.** No high-speed USB 2.0 on the Pico. Fine for HID; insufficient for video streaming, but we don't.
- **Single boot-mouse / boot-keyboard** at a time. Windows accepts one of each composite device. Don't plug in multiple Synapse boards.
- **Gamepad compatibility.** The Pico exposes a standard HID gamepad, not an
  XInput device. Use ViGEm for XInput-only games.
- **Mouse resolution.** 16-bit signed delta per axis. Large moves split into many small reports anyway.
- **Latency stable on Win11 22H2+.** Older Windows builds may have USB poll jitter (1 ms poll effectively 1-3 ms). Not Synapse's problem to fix.
- **PIO USB host (advanced).** The Pico's PIO blocks can run a second USB host port (see `vynxc/VBox`). Synapse v1 doesn't ship this; v2 option for "pass through a real mouse and inject corrections."

---

## 13. What this doc does NOT cover

- Supported-use policy and hardware permission gates → `08`
- High-level action API routing to hardware → `03_action.md`
- Action serialization invariants → `03_action.md` §4
- Build pipeline / installer integration → `14_build_and_packaging.md`
