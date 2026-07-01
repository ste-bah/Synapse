# Build Speed And Disk Hygiene

This repo is tuned for fast iterative builds on the configured Windows dev host
and to prevent git worktree plus Cargo `target/` buildup from consuming the
system disk again.

## Build Settings

| Change | Where | Effect |
| --- | --- | --- |
| `rust-lld` linker | `.cargo/config.toml` | Replaces slow MSVC `link.exe` for faster Windows linking. Missing linker state fails loudly. |
| Dev incremental compilation | `Cargo.toml [profile.dev]` | Rebuilds only changed codegen units during the edit loop. |
| Dependency debuginfo off | `Cargo.toml [profile.dev.package."*"]` | Avoids large dependency debug artifacts while keeping workspace panic line tables. |
| `jobs = 32` | user Cargo config | Uses the configured host's logical cores for local builds. |

Use the fast local edit loop:

```powershell
cargo check
cargo build
```

Use `cargo build --release` only when shipping or running the optimized daemon,
not as compile feedback during edits.

## Defender Exclusions

Defender real-time scanning of Cargo output can slow local builds. The helper
self-elevates and adds the repo build-output exclusions:

```powershell
pwsh -File .\scripts\add-defender-exclusions.ps1
```

## Repo Maintenance

`scripts/repo-maintenance.ps1` is dry-run by default. It scans git repos under
`-Root` (default `C:\code`) and:

- prunes merged or remote-gone worktrees without touching active, dirty, or
  unmerged work;
- deletes local branches whose remote branch is gone;
- runs `cargo sweep` for stale build artifacts.

```powershell
pwsh -File .\scripts\repo-maintenance.ps1
pwsh -File .\scripts\repo-maintenance.ps1 -Apply
```

`scripts/install-maintenance-task.ps1` registers or removes the weekly
non-elevated Scheduled Task:

```powershell
pwsh -File .\scripts\install-maintenance-task.ps1
pwsh -File .\scripts\install-maintenance-task.ps1 -Remove
Get-ScheduledTask -TaskName SynapseRepoMaintenance
```

## Root Cause

Parallel issue work created many throwaway git worktrees, each with its own
multi-GB Cargo `target/`. Git does not automatically remove worktrees, and Cargo
does not garbage-collect old `target/` artifacts. Scheduled worktree pruning plus
`cargo sweep` keeps the checkout set and build artifacts bounded.
