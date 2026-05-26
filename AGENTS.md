# Synapse Agent Doctrine

This repository uses GitHub Issues as the coordination and state surface. Read
the issue queue before changing code, and treat `status:in-progress` issues
assigned to this agent as resumable work after context compaction.

## Non-Excusable FSV Rule

Full State Verification (FSV) must be performed manually by the AI agent. It
must never be delegated to a script, test, benchmark, harness, CI job, GitHub
Action, or any other automated substitute.

For every shipped change, the agent must:

1. Define the Source of Truth (SoT): database/table/key, file path, queue,
   metric, global state, external system record, or UI state.
2. Read the SoT before the trigger.
3. Execute the trigger manually with synthetic inputs whose expected outputs
   are known.
4. Read the SoT again with a separate operation and record the actual state.
5. Manually exercise the happy path plus at least three edge cases, printing
   before and after state for each.

Automated tests, property tests, benchmarks, scripts, and build checks are
supporting regression evidence only. They are not FSV and must not be named or
presented as FSV. Do not add new `*_fsv` tests, FSV harnesses, or FSV scripts.
When Synapse exposes a real runtime surface for the behavior under review,
including MCP tools or daemon endpoints, manual FSV must trigger that real
surface and then inspect the separate physical source of truth/state it
produced. Scripts must not stand in for that runtime trigger or source-of-truth
readback.

## No GitHub Actions / CI Gate

Do not dispatch, wait on, or use GitHub Actions/CI as a shipping gate unless a
later explicit operator decision reverses issue #351. Agent commits pushed to
this repo must include `[skip ci]`.

## Missing Dependency Acquisition Rule

If a required local tool, driver, model, device, file, service, account state,
installer, hardware surface, or other prerequisite is missing, do not treat the
absence as a stopping point or mark the issue blocked for that reason alone.
Missing means: figure out where the thing must come from, where it must
physically appear, and make it happen on this configured host. Synapse gives the
agent local computer control; treat those Synapse/local control surfaces as the
operator-equivalent host control surface. If the operator could download,
install, connect, configure, generate, flash, launch, or inspect it from this
host, the agent must attempt those reversible local steps using Synapse plus
normal OS, shell, browser, package-manager, and device-management workflows.
Do not ask the operator to download or install something while reversible local
acquisition/setup remains possible.

Operationally: do not stop at "missing." Use the same local host control the
operator has through Synapse. If it can be done from this computer, do it and
then inspect the resulting source of truth. Missing configured-host state is
never a blocker by itself.

Every reversible host action the operator could perform from this computer is
agent-owned work: browser downloads, GUI installers, Device Manager checks,
package-manager installs, model/file generation, firmware flashing, launching
apps, and UI inspection through Synapse. "Blocked" is reserved for the exact
operator-only external action that remains after those local steps are
exhausted.

Missing prerequisites become concrete work items with source-of-truth readback:
identify what is missing, identify the authoritative place it should appear,
perform the acquisition/setup step, then read that source of truth directly. If
the next step requires a hard-to-reverse external action such as spending money,
using private credentials, changing billing, or modifying an external account,
prepare the exact action and ask only for that approval. Complete every
reversible local step before asking. Do not call the issue done until the real
prerequisite and the feature that depends on it are manually verified at the
physical source of truth.

## Required Wake-Up Context

After compaction or a new session, re-read:

1. `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
2. This file
3. Open and closed GitHub decision/context issues, especially #351
4. `git status` and the active issue comments
