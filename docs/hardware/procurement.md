# Hardware Procurement

This page covers the physical hardware and host setup needed for the M4 Pico
HID path. Missing hardware is acquisition/setup work, not a reason to mark a
work item blocked by absence alone. Complete every reversible local step first:
verify the toolchain, build the UF2, inspect USB/device state, and only then ask
for narrow approval if buying hardware or using an external account is required.
Synapse gives the agent full local computer-control responsibility on this
host, so browser sourcing, installer downloads, tool installs, BOOTSEL/USB/COM
inspection, firmware flashing, and UI inspection are agent-owned while they are
locally reversible.
Missing local state creates the next action for the agent, not a blocker while
reversible host work remains.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.

## Recommended board

Use an RP2040-based Raspberry Pi Pico for M4. The first choice is:

| Item | Why | Notes |
|---|---|---|
| Raspberry Pi Pico H, order code `SC0917` | RP2040 target with pre-soldered headers | Best default for this project. The headers are not required for the USB-only M4 path, but they avoid soldering for future lab work. |
| Raspberry Pi Pico WH, order code `SC0919` | Same RP2040 class with pre-soldered headers and wireless hardware | Acceptable if Pico H is unavailable. Wi-Fi/Bluetooth are not used by Synapse M4. |
| Raspberry Pi Pico, order code `SC0915` | Same RP2040 class without headers | Acceptable for USB-only flashing, but do not choose it if the operator wants no soldering for future GPIO work. |

Do not use Pico 2 / Pico 2 H as the first M4 board. Those are RP2350 boards,
while the current firmware target is RP2040 (`thumbv6m-none-eabi`) and M4
acceptance is written for the RP2040 Pico path.

For first-generation Pico boards, Raspberry Pi's Pico-series documentation
lists Pico H and Pico WH as the pre-soldered-header variants.

## USB cable

Use a data-capable USB cable. A charge-only cable can power the Pico but will
not enumerate `RPI-RP2`, a COM port, or the final composite device.

For Pico / Pico H / Pico W / Pico WH, the board connector is micro-USB. Use a
known data cable and a direct motherboard port when doing first FSV. Avoid
unpowered hubs until the board has already been verified.

## Where to source

As of 2026-05-26, expected US single-unit pricing for Pico H is about `$5`
before tax and shipping. Verify stock and final price before purchasing.

| Source | Part | Readback on 2026-05-26 |
|---|---|---|
| Mouser | `358-SC0917` / `SC0917` | Listed Raspberry Pi Pico H, in stock, unit price `$5.00`. |
| DigiKey | `2648-SC0917-ND` / `SC0917` | Listed Raspberry Pi Pico H, in stock, unit price `$5.00000`. |
| Adafruit | Product `5525` | Lists Pico H at `$5.00`; pre-soldered-header option may be out of stock. |

If all preferred sources are out of stock, find an authorized Raspberry Pi
reseller with `SC0917` or `SC0919`. Avoid clone boards for the first M4 FSV
because VID/PID, bootloader behavior, and USB descriptors can differ.

## Host toolchain

Run these on the configured Windows host:

```powershell
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs
```

Read the toolchain SoT directly:

```powershell
rustup target list --installed
cargo install --list
```

Expected readback:

- `thumbv6m-none-eabi` appears in the installed Rust target list.
- `elf2uf2-rs` appears in the installed Cargo binary list.

## Build the firmware

```powershell
cd C:\code\Synapse\firmware\pico-hid
cargo build --release
elf2uf2-rs target\thumbv6m-none-eabi\release\pico-hid pico-hid.uf2
```

Read the file SoT directly:

```powershell
Get-Item .\pico-hid.uf2
Get-FileHash .\pico-hid.uf2 -Algorithm SHA256
```

Expected readback:

- `pico-hid.uf2` exists in `firmware\pico-hid`.
- File length is nonzero.
- SHA-256 hash is printed and recorded in the active issue comment.

## Flash with BOOTSEL

1. Hold the Pico `BOOTSEL` button.
2. While holding `BOOTSEL`, plug the Pico into the host with the data cable.
3. Release `BOOTSEL` after Windows mounts the mass-storage volume.
4. Verify the `RPI-RP2` volume exists.
5. Copy `pico-hid.uf2` to that volume.
6. Wait for the volume to disappear and the device to reboot.

PowerShell readback before copying:

```powershell
Get-CimInstance Win32_LogicalDisk |
  Where-Object VolumeName -eq 'RPI-RP2' |
  Select-Object DeviceID,VolumeName,Size,FreeSpace
```

Copy once a drive letter is known:

```powershell
Copy-Item .\pico-hid.uf2 E:\
```

Replace `E:` with the actual `DeviceID` from the readback.

Read the post-flash hardware SoT:

```powershell
Get-PnpDevice -PresentOnly |
  Where-Object { $_.FriendlyName -match 'Pico|RP2040|Synapse|USB Serial|HID' } |
  Select-Object Status,Class,FriendlyName,InstanceId

Get-CimInstance Win32_SerialPort |
  Select-Object DeviceID,Name,PNPDeviceID
```

The first firmware milestone blink image should blink the onboard GP25 LED once
per second. Later M4 images should enumerate as the Synapse composite HID + CDC
device and answer `hid identify`.

## References

- Raspberry Pi Pico-series docs:
  <https://www.raspberrypi.com/documentation/microcontrollers/raspberry-pi-pico.html>
- Mouser SC0917:
  <https://www.mouser.com/ProductDetail/Raspberry-Pi/SC0917>
- DigiKey SC0917:
  <https://www.digikey.com/en/products/detail/raspberry-pi/SC0917/16608257>
- Adafruit product 5525:
  <https://www.adafruit.com/product/5525>
