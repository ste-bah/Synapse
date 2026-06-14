# Build speed & disk hygiene (read this before you `cargo build`)

Canonical doctrine lives in `AGENTS.md` **D5**; this is the detailed reference.

## Match the build to the task — do NOT `--release` to test a code change

| Command | Use it for | Typical time |
|---------|-----------|--------------|
| `cargo check` | "does it compile?" — fastest feedback, most edits | ~10–20 s |
| `cargo build` (dev) | a runnable debug binary | clean ~1m46s, **incremental edit ~45 s** |
| `cargo build --release` | ship/run the optimized `synapse-mcp.exe` daemon | clean ~3m25s |
| `cargo build --profile release-max` | the absolute-max-runtime binary (rare) | slow (fat LTO) |

The #1 mistake that makes builds "take forever": running `--release` (or the installer) on every edit. Release is for shipping the daemon, not for compile feedback. Iterate with `cargo check` / `cargo build`.

## Why builds are fast now (config, host = Ryzen 9 9950X3D 16C/32T, 128 GB)

- **Linker = `rust-lld`** (`.cargo/config.toml`): replaces the slow MSVC `link.exe`; cuts link time ~2–5×. Bare name auto-resolves from the toolchain sysroot (tracks `rustup update`); no silent fallback.
- **dev `incremental = true`** + dependency debuginfo dropped (`Cargo.toml [profile.dev]`).
- **`jobs = 32`** (`~/.cargo/config.toml`, not committed): one job per logical thread.
- **`release` = `lto="thin"` + `codegen-units=16` + `debug=false`** (was `fat`/`1`, which serialized the final optimization through ~1 thread and idled 28 of 32 cores → 15–30 min builds). Near-identical runtime for this I/O/perception daemon (RocksDB is C++, unaffected by Rust LTO). The old config is preserved as `--profile release-max`.

### Windows-specific tax: antivirus
Microsoft Defender real-time scanning inspects every artifact the compiler writes. If builds are mysteriously slow, check `Get-MpComputerStatus` (`RealTimeProtectionEnabled`). Either disable real-time protection or run `scripts/add-defender-exclusions.ps1` (self-elevating; excludes `C:\code`, `~/.cargo`, `~/.rustup`, compiler procs).

## Disk hygiene — why it stays flat

Parallel/per-issue agent worktrees (`Synapse-issueNNN-clean`) each get their own multi-GB `target/`. Git never removes worktrees and Cargo never GCs `target/`, so they accumulate (on 2026-06-14: 46 worktrees, ~200 GB, disk at 94 % — which itself starved builds). Prevention:

- `scripts/repo-maintenance.ps1` — dry-run by default, `-Apply` to act. Backs up then prunes merged/stale worktrees (squash-merge aware), deletes `[gone]` branches, `cargo-sweep`s stale `target/` artifacts. Run `pwsh -File scripts/repo-maintenance.ps1` to preview anytime.
- `scripts/install-maintenance-task.ps1` — registers the weekly Scheduled Task `SynapseRepoMaintenance` (non-elevated). `synapse-update.ps1` auto-ensures it.
- **Rule for agents:** never leave a throwaway `git worktree` (with its own `target/`) around after the issue lands — remove it (`git worktree remove`). `origin/main` is the single canonical branch; base new work on it.

Requires `cargo install cargo-sweep`. Scripts are PowerShell 7 — run with `pwsh`, not `powershell` (5.1).
