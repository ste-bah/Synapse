# 08 — Supported Use Policy

## 1. Purpose of this doc

This is a **policy** doc, not a technical doc. It defines the use cases Synapse is built to support, the capabilities contributors must keep out of scope, and the operator confirmations required before sensitive local-control features are enabled.

This policy is binding on contributors. PRs that violate it are rejected. Defaults are conservative: Synapse should work well for local computer use, accessibility tooling, research rigs, sanctioned game-AI experiments, QA automation, and single-player game control without quietly enabling unrelated or high-impact behavior.

---

## 2. The single rule

> **Synapse is local computer-control infrastructure. It should help an operator or an explicitly authorized agent see, hear, act on, and react inside software the operator is allowed to automate. It should not add features whose primary purpose is raw manipulation of third-party processes, unsupported device-identity changes, unregistered persistence, or scaled unattended account operation.**

Hardware HID, natural cursor curves, virtual controllers, fast capture, and reflexes are ordinary local-control primitives. They exist for accessibility, QA, game-AI research, local demos, simulation rigs, and single-player play.

---

## 3. Supported contexts

Profiles declare a `use_scope`. The field is descriptive metadata for permission checks and user-facing warnings.

| Scope | Examples | Default posture |
|---|---|---|
| `productivity` | Notepad, VS Code, Chrome, Slack, Discord, terminals, File Explorer | Actions allowed according to normal tool permissions |
| `single_player` | Minecraft Java local worlds, Factorio, Stardew Valley, Skyrim, KSP, OpenTTD | Game actions allowed; hardware HID remains opt-in |
| `operator_owned_test` | QA fixtures, private test servers, local simulators, replay harnesses | Actions allowed when the profile declares the test boundary |
| `sanctioned_research` | University game-AI rigs, AI tournaments, benchmark environments | Actions allowed with explicit profile metadata and operator setup |
| `unknown` | New games/apps without a reviewed profile | Observation allowed; action defaults should be minimal until the profile is reviewed |

The profile loader rejects unknown `use_scope` values. Bundled profiles must include `use_scope` and a short comment describing the intended environment.
Bundled benchmark profiles must also expose metadata gates such as
`supported_use.local_world_only`, `supported_use.approved_worlds`, and
`supported_use.remote_server_allowed` through `profile_list`. Those metadata
keys are the profile registry's source of truth until a runtime target-policy
checker is added.

---

## 4. Capability boundaries

### 4.1 Frozen capabilities

These capabilities stay disabled unless an ADR explicitly changes the project scope:

1. **DLL injection into any process.** Synapse does not load code into target applications.
2. **Raw process memory read/write tooling** for other processes. Game-provided or app-provided APIs are acceptable when documented by the application owner.
3. **Kernel driver hooks.** Synapse is user-mode only. No `.sys` files in the install.
4. **Graphics-pipeline injection.** Capture uses Windows capture APIs, not injected hooks.
5. **Custom device-identity firmware in release builds.** Bundled firmware uses the Synapse Pico HID VID/PID from ADR-0008 and does not ship unrelated commercial device IDs.
6. **Unregistered persistence.** Synapse identifies itself plainly in process names, logs, metrics, and device identity strings.
7. **Automatic escalation based on foreground app.** A profile match may select defaults, but it must not silently elevate permissions beyond the operator's startup configuration.

### 4.2 Sensitive but supported capabilities

These capabilities ship because they are useful for legitimate local automation. They remain explicit and auditable:

1. **Natural cursor curves and keystroke pacing.** Useful for accessibility, demos, QA, and smoother game control.
2. **Hardware HID gateway.** Useful for accessibility adapters, simulation rigs, dedicated game-AI research machines, and hardware testing.
3. **Graphics Capture API and DXGI Output Duplication.** Standard Windows capture paths.
4. **WASAPI loopback audio capture.** Standard Windows audio loopback.
5. **WinEvent / UIA event subscribers.** Standard Windows accessibility APIs.
6. **Chrome DevTools Protocol attachment.** Public browser API when the browser is configured for it.
7. **Filesystem and process watchers.** Standard Windows APIs, subject to redaction and permissions.

---

## 5. Operator responsibility

By installing and configuring Synapse, the operator acknowledges:

- They are responsible for using automation only in environments where they have authorization.
- Synapse can move input devices, type text, launch processes, read visible content, and store replay logs.
- Sensitive capabilities are opt-in and logged.
- The project provides tooling and safety defaults, not legal or organizational approval for a specific deployment.

First-run prompt:

```
Synapse is a local computer-control tool. By continuing you confirm:

1. You will use Synapse only where you are authorized to automate.
2. You understand Synapse can type, click, launch processes, capture visible
   screen content, capture system audio, and store replay logs.
3. You will enable sensitive capabilities only when they are needed for your
   local workflow, research rig, accessibility setup, QA environment, or
   single-player/sanctioned game profile.

Type 'i agree' to continue. (Decline by closing this prompt.)
```

Acknowledgment is recorded in `%APPDATA%\synapse\agreement.json` with a hash of the prompt text and a timestamp. A new major version may invalidate the previous acknowledgment.

Hardware HID has a separate first-use confirmation because it physically injects
keyboard, mouse, and gamepad input into the OS. Starting with
`--hardware-hid <port|auto>` on a configured host prompts for the exact phrase
`I AUTHORIZE HARDWARE INPUT` before `%APPDATA%\synapse\agreement.json` is
written. Any other response exits with
`SAFETY_PROFILE_ACTION_DENIED reason=hardware_consent_refused` and leaves the
agreement file absent. Subsequent runs skip the prompt after the agreement file
validates; `--reset-hardware-consent` deletes the existing agreement and
requires the phrase again.

---

## 6. Permission responses

When an action is about to fire, the MCP layer checks session permissions, profile metadata, backend availability, and startup flags before dispatch.

| Situation | Default behavior | Operator override |
|---|---|---|
| `use_scope = "unknown"` and a write/action tool is requested | Refuse with `SAFETY_PROFILE_ACTION_DENIED`, log event | Activate a reviewed profile or pass an explicit profile override |
| Hardware HID requested without hardware enabled | Refuse with `ACTION_BACKEND_UNAVAILABLE` | Start with `--hardware-hid <port|auto>` |
| Audio tool requested without audio enabled | Refuse with `SAFETY_PERMISSION_DENIED` | Start with `--enable-audio` or set `SYNAPSE_ENABLE_AUDIO=true` |
| Launch process requested outside allowlist | Refuse with `SAFETY_LAUNCH_DENIED_BY_POLICY` | Add `--allow-launch <regex>` or config entry |
| Shell command requested outside allowlist | Refuse with `SAFETY_SHELL_DENIED_BY_POLICY` | Add `--allow-shell <regex>` or config entry |
| Redaction disabled | Requires startup flag and first-use confirmation | `--no-redaction` |
| Non-loopback HTTP bind | Requires startup flag and first-use confirmation | `--bind <addr>` |

The checks gate Synapse's own behavior. They do not inspect or classify third-party protection systems.

---

## 7. Specific guidance for likely v1 profiles

### 7.1 Productivity profiles

Notepad, VS Code, Chrome, Slack, Discord, Terminal, and File Explorer are `productivity` profiles. They prefer accessibility APIs and semantic invocation over coordinate motion whenever possible.

### 7.2 Minecraft Java Edition

`single_player`. Recommended first game profile. HUD extraction, keymap, entity detection, and reflex demos target a local world.

### 7.3 Luanti / Minetest Game Benchmark

`operator_owned_test`. The bundled `luanti.minetest` profile is restricted to
the local configured-host benchmark install and approved local benchmark worlds.
Its metadata declares the launch target, benchmark world, and
`supported_use.remote_server_allowed = "false"` so profile-registry/audit FSV
can verify the intended boundary before actions run.

### 7.4 Factorio

`single_player` or `operator_owned_test` depending on setup. Headless support exists through Factorio's own interfaces; Synapse's GUI driving is supplementary.

### 7.5 OpenTTD, BeamNG, KSP, RimWorld, Stardew Valley

`single_player`. Suitable for bundled or community profiles after normal smoke tests.

### 7.6 Browser games and Roblox Studio

Browser games use the Chrome profile machinery. Roblox Studio is `operator_owned_test`. Runtime experiences should start as `unknown` until a profile states the intended environment.

---

## 8. Updating profile scopes

Changing a bundled profile's `use_scope` is a release-visible change. It requires:

1. A short changelog entry.
2. A profile smoke test update.
3. Documentation of the intended environment in the profile comment.

Synapse does not auto-update profiles without operator consent.

---

## 9. What this doc does NOT cover

- Hardware HID firmware design → `09_hardware_hid_gateway.md`
- Action back-end mechanics → `03_action.md`
- Per-tool permission requirements → `05_mcp_tool_surface.md`
- Redaction, network binding, and local trust boundaries → `11_security_and_safety.md`
