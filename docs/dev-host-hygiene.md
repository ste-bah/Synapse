# Dev-host hygiene

Recommendations for keeping a long-lived synapse development host (Windows or Linux) lean and predictable. None of this is enforced at build time; it's collected here so contributors don't have to rediscover the trade-offs each time.

## Missing configured-host prerequisites

If a required host prerequisite is absent, treat that as setup/acquisition work,
not as completion evidence, a portability gate, or a reason to mark the issue
blocked by itself. Missing means: figure out where the thing must come from,
where it must physically appear, and make it happen on this configured host.
Synapse gives the agent full local computer-control responsibility, so treat
Synapse/local control as the operator-equivalent host control surface. If the
operator could download, install, connect, configure, generate, flash, launch,
or inspect it from this host, the agent must attempt those reversible local
steps using Synapse plus normal OS, shell, browser, package-manager, and
device-management workflows.
Do not ask the operator to download or install something while reversible local
acquisition/setup remains possible.
Do not stop at "missing." Synapse gives the agent the same practical local
ability the operator has at this keyboard. If the operator could make it happen
from this computer, the agent must use Synapse and local host workflows to do
the reversible work, then inspect the resulting source of truth. Missing
configured-host state is never a blocker by itself.
Missing local state creates the next action for the agent and must be made
real, not handed back to the operator or treated as a blocker while reversible
host work remains.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.
That includes browser downloads, GUI installers, Device Manager checks,
package-manager installs, model/file generation, firmware flashing, app
launching, USB/COM inspection, and UI inspection through Synapse. Treat those
as agent-owned work, not operator errands. Do not hand local prerequisite work
back to the operator while it remains reversible from this computer.
Then read the physical source of truth where the prerequisite must appear:
package inventory, driver/service list, `Get-PnpDevice`, registry Enum key,
firmware volume, config file, model path plus hash, or equivalent.

Ask for narrow operator approval only before hard-to-reverse external actions:
spending money, using private credentials, changing billing, modifying an
external account, or irreversible shared-state changes.
Complete every reversible local step before asking for that approval.

## MCP runtime hygiene for FSV

Do not assume a prior `synapse-mcp` process is still valid after compaction,
repo changes, or client transport errors. Before accepting Synapse behavior,
read the real runtime state: process table or stdio child, executable path and
hash when relevant, loopback bind/socket or client transport, authenticated
`health`, initialized MCP session, and `tools/list` containing the required
tool.

If the configured chat MCP reports `Transport closed`, treat that as client
transport state. Launch or reinstall the repo-built runtime, preferably with an
issue-local `.runs/<run-id>/db` and log directory for shipping evidence, then
repeat the readback. FSV triggers Synapse behavior through the real MCP
`tools/call` when a tool exists, and the verdict comes from a separate physical
SoT read after the call.

## Coordinates: `act_aim` / `act_click({x,y})` / `act_drag` / `act_scroll`

Synapse interprets all `{x, y}` mouse coordinates as **physical (DPI-aware) pixels** — the same units `GetCursorPos` returns from a per-monitor-DPI-aware process, and the same units UI Automation bounding boxes use. This matches the daemon's own DPI awareness (synapse-mcp is built as per-monitor V2) and `mouse_coordinates.rs::normalize_absolute_mouse_point` which feeds `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`.

Source-of-truth verifiers must use a DPI-aware reader or they will read **logical** (virtualised) coords and disagree with synapse by the monitor scale factor. The most common gotcha: PowerShell 5.1's `[System.Windows.Forms.Cursor]::Position` is DPI-unaware by default. The fix in PowerShell:

```powershell
Add-Type @'
using System;
using System.Runtime.InteropServices;
public struct POINT { public int X; public int Y; }
public class W { [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
                 [DllImport("user32.dll")] public static extern int SetProcessDPIAware(); }
'@
[W]::SetProcessDPIAware() | Out-Null
$p = New-Object POINT; [W]::GetCursorPos([ref]$p) | Out-Null
"$($p.X),$($p.Y)"
```

The `mouse_coordinates.rs::tests` cases double as a reference for the expected mapping on multi-monitor setups.

Element-id-based targets (`act_click({element_id: …})`) are unaffected — UIA bboxes are already in the synapse-native coord space.

## Release-profile build OOM on Windows (#236)

The default `[profile.release]` plus a full-workspace install can blow past 32 GB of pagefile commit during the link step, especially on multi-DLL Windows targets. Three independent amplifiers:

1. **Fat LTO over many crates.** `lto = "fat"` requires whole-program optimisation in a single unit; combined with the synapse-mcp dep graph (rmcp + chromiumoxide + uiautomation + windows-capture + perception) it routinely exceeds 32 GB commit.
2. **Default `CARGO_BUILD_JOBS`.** Without the env, cargo defaults to `num_cpus`; that multiplies the spike across cores.
3. **WSL-mounted `target/`.** Every artifact write hits 9P and amplifies memory pressure.

### Recommended install recipe for 32 GB workstations

```powershell
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_TARGET_DIR  = 'C:\cargo-target\synapse'   # native NTFS, not WSL
cargo install --path crates/synapse-mcp --force --locked
```

Optional `Cargo.toml` adjustment (not yet applied; tracked under #236):

```toml
[profile.release]
codegen-units = 4   # was 16; 4 trades a bit of build time for ~40-60% lower peak commit
lto           = "thin"
# strip       = "debuginfo"   # if you don't need backtraces for release crashes
```

For very link-heavy builds, swapping the linker:

```toml
[target.x86_64-pc-windows-msvc]
linker    = "rust-lld.exe"
rustflags = ["-Clink-arg=/STACK:16777216"]
```

### Pagefile guidance

Microsoft's official recommendation is system-managed pagefile for dev workstations. If you keep manual sizing, target ≥ `2 × RAM` and put the file on the same drive as `target/`. The 16 / 32 GB cap on a 32 GB-RAM host has been observed to OOM the release link of this workspace.

## `target/` directory hygiene (#240)

Cargo never garbage-collects `target/`; on this repo it routinely reaches 8-10 GB on a single dev host. Use [`cargo-sweep`](https://github.com/holmgr/cargo-sweep) for selective pruning that preserves the incremental cache for files you touch:

```powershell
cargo install cargo-sweep
cargo sweep -t 30           # delete artifacts older than 30 days
cargo clean --doc            # docs alone can hit 1 GB+
cargo clean -p chromiumoxide --release   # surgical purge of the largest single dep
```

For multi-repo developers, set a shared cache in `~/.cargo/config.toml`:

```toml
[build]
target-dir = "C:/cargo-target/shared"
```

Be aware that `cargo clean` then nukes the shared cache for every repo using it.

`CARGO_INCREMENTAL=0` is the default for `release`; set it explicitly in install scripts to override stray env vars from CI tooling.

## Ephemeral Operator-Run Output (#242)

The canonical location for ad-hoc run artifacts (`.log`, `.ndjson`, scratch JSON, debug screenshots) is:

* **Repo-local:** `./.runs/<run-id>/<file>` — easy to grep / diff / reference from issue comments
* **OS-cache (Windows):** `%LOCALAPPDATA%\synapse\runs\` — never pollutes git, survives `git clean -fdx`
* **OS-cache (Linux/macOS):** `$XDG_CACHE_HOME/synapse/runs/` (fallback `~/.cache/synapse/runs/`)

Legacy repo-root verification directories should be migrated to `.runs/<id>/` or deleted.

`scripts/clean-runs.ps1` prunes `.runs/` subdirs older than 30 days by default.

```powershell
.\scripts\clean-runs.ps1
```

## Benchmark baselines (#243 / #260 / #350)

Do not commit Criterion baselines or raw benchmark exports. `bench_results/` is gitignored, and benchmark state is stored off-tree:

* Durable release/tag baselines: `%LOCALAPPDATA%\synapse\benchmarks\baselines\`
* Per-run candidate exports and manual evidence notes: `.runs\benchmarks\<run-id>\`

Use local Criterion baselines plus [`critcmp`](https://github.com/BurntSushi/critcmp); per #351, do not use GitHub Actions/CI or Bencher as a shipping gate unless a later operator decision explicitly reverses this. Benchmark scripts are supporting evidence only, never FSV.

```powershell
cargo install critcmp
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\synapse\benchmarks\baselines" ".runs\benchmarks" | Out-Null
cargo bench --workspace --benches -- --save-baseline main
critcmp --export main > "$env:LOCALAPPDATA\synapse\benchmarks\baselines\main.json"

cargo bench --workspace --benches -- --save-baseline candidate
critcmp --export candidate > ".runs\benchmarks\candidate.json"
critcmp "$env:LOCALAPPDATA\synapse\benchmarks\baselines\main.json" ".runs\benchmarks\candidate.json"
.\scripts\check-bench-delta.ps1 -BaselineJson "$env:LOCALAPPDATA\synapse\benchmarks\baselines\main.json" -CandidateJson ".runs\benchmarks\candidate.json"
```

`scripts/check-bench-delta.ps1` is the local 20% regression gate over exported `critcmp` JSON. It fails when a tracked benchmark is missing from the candidate export or when the candidate mean is more than 20% slower than the baseline. Manual evidence still reads the export JSON and command output directly; the script is a local comparator, not a substitute for source-of-truth inspection.
