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

Before any Synapse behavior is accepted by FSV, the agent must prove the real
`synapse-mcp` daemon is running and active on this host. Read the process/socket
source of truth, authenticate to the daemon, call `health`, initialize an MCP
session, and read `tools/list` so the required tool is physically present. If
the daemon is absent, stale, or unreachable, launching or reinstalling the
repo-built runtime is the next local setup action. For any behavior with an MCP
tool, the trigger must be the real `tools/call`; a CLI, unit test, helper
binary, script, or direct storage write may only support investigation and must
not replace the MCP trigger. FSV evidence must name the daemon PID/bind or stdio
child, session/tool used, and the separate SoT read after the tool call. A
`health` response or tool return value is not the verdict by itself.

## Delta-Of-Reality Operating Rule

Synapse should move toward a delta-first reality model tracked by issue #536.
After a baseline snapshot, routine agent context should be ordered changes in
reality, not repeated full snapshots. Long-running work must periodically ask
Synapse to audit the accumulated assumption against physical reality and force
a rebase when drift is detected. Until the delta tools are implemented, agents
must continue using the existing real MCP tools and separate SoT readbacks, and
must file or update the #536 child issues instead of letting delta-reality work
live only in chat context.

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
agent full local computer-control responsibility; treat those Synapse/local
control surfaces as the operator-equivalent host control surface. If the
operator could download, install, connect, configure, generate, flash, launch,
or inspect it from this host, the agent must attempt those reversible local
steps using Synapse plus normal OS, shell, browser, package-manager, and
device-management workflows.
Do not ask the operator to download or install something while reversible local
acquisition/setup remains possible.

Operationally: do not stop at "missing." Use the same local host control the
operator has through Synapse. For reversible local work, Synapse gives the
agent the same practical ability the operator has at this keyboard. If it can
be done from this computer, the agent must do it and then inspect the resulting
source of truth. Missing configured-host state is never a blocker by itself.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.

Treat missing local state as the next action and make it happen, not as an
excuse to pause or ask the operator to do local setup. Synapse gives enough
local computer control to do every reversible host action the operator could do
from this machine. Use that control to keep the issue moving until the only
remaining step is a specific operator-only external decision.

Synapse gives the agent full local computer-control responsibility for this
host. Do not hand local work back to the operator just because a prerequisite is
absent. Every reversible host action the operator could perform from this
computer is agent-owned work: browser downloads, GUI installers, Device Manager
checks, package-manager installs, model/file generation, firmware flashing,
launching apps, USB/COM inspection, and UI inspection through Synapse.
"Blocked" is reserved for the exact operator-only external action that remains
after those local steps are exhausted.

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
