# Issue 1697 EOL Policy FSV - 2026-07-15

Issue: https://github.com/ChrisRoyse/Synapse/issues/1697

This records manual Full State Verification for the repository line-ending
policy. No automated tests were created or run for this issue.

## Research Inputs

- Git `gitattributes` documentation: `text` normalizes line endings in the
  index, `eol` controls checkout line endings, and unspecified attributes fall
  back to `core.autocrlf` / `core.eol`.
- GitHub line-ending documentation: a committed `.gitattributes` overrides
  contributor-local line-ending config and `git add --renormalize .` is the
  deliberate normalization step after policy changes.

## Source Of Truth

- `.gitattributes`
- `git check-attr text eol diff merge -- <path>`
- `git ls-files --eol <path>`
- `git -c core.autocrlf=true diff --check`
- Physical file bytes on disk after a fresh checkout

## Root Cause

The repo only pinned `*.sh` and `.githooks/*` to LF. All ordinary text files
fell back to `core.autocrlf`; on this Windows host `core.autocrlf=true`, so
Rust/TOML/Markdown/PowerShell files checked out as CRLF and `git diff --check`
could emit conversion warnings even when there was no semantic change.

## Fix

- Default repository text to `text=auto eol=lf`.
- Explicitly pin scripts, Rust/CUDA, TOML/lockfiles, Markdown, JSON/YAML,
  HTML/CSS/JS/TS, snapshots, and other tracked text formats to LF.
- Preserve Windows-only `.bat`, `.cmd`, and `.sln` as CRLF exceptions.
- Mark binary assets as binary and unset inherited `eol`.
- Mark `dashboard/dist/**` as generated output with `text` and `eol` unset.
- Document the policy and manual readback commands in
  `docs/BUILD-AND-MAINTENANCE.md`.

## Before State

Observed before the fix in a clean worktree with `core.autocrlf=true`:

```text
.gitattributes:
*.sh text eol=lf
.githooks/* text eol=lf

git ls-files --eol .githooks/pre-push .gitattributes scripts/synapse-setup.ps1 Cargo.toml crates/synapse-mcp/src/main.rs docs/BUILD-AND-MAINTENANCE.md tests/fixtures/audio/hello_world_5s.wav
i/lf    w/crlf  attr/                  .gitattributes
i/lf    w/lf    attr/text eol=lf       .githooks/pre-push
i/lf    w/crlf  attr/                  Cargo.toml
i/lf    w/crlf  attr/                  crates/synapse-mcp/src/main.rs
i/lf    w/crlf  attr/                  docs/BUILD-AND-MAINTENANCE.md
i/lf    w/crlf  attr/                  scripts/synapse-setup.ps1
i/-text w/-text attr/                  tests/fixtures/audio/hello_world_5s.wav
```

There were `2497` tracked paths with `w/crlf` or `w/mixed`.

## After Attribute Readback

After editing `.gitattributes` and running `git add --renormalize .`, only
`.gitattributes`, `docs/BUILD-AND-MAINTENANCE.md`, and this FSV document are
part of the change.

```text
git -c core.autocrlf=true diff --check
git diff --cached --check
```

Both commands exited with code `0` and no output.

Attribute readback:

```text
.githooks/pre-push: text: set
.githooks/pre-push: eol: lf
scripts/synapse-setup.ps1: text: set
scripts/synapse-setup.ps1: eol: lf
Cargo.toml: text: set
Cargo.toml: eol: lf
crates/synapse-mcp/src/main.rs: text: set
crates/synapse-mcp/src/main.rs: eol: lf
README.md: text: set
README.md: eol: lf
docs/BUILD-AND-MAINTENANCE.md: text: set
docs/BUILD-AND-MAINTENANCE.md: eol: lf
dashboard/dist/index.html: text: unset
dashboard/dist/index.html: eol: unset
tests/fixtures/audio/hello_world_5s.wav: text: unset
tests/fixtures/audio/hello_world_5s.wav: eol: unset
```

## Fresh Checkout Byte Readback

Verification worktree:
`C:\code\Synapse-issue1697-verify`

The verification worktree was a fresh checkout of the issue commit with
`core.autocrlf=true`.

```text
git -c core.autocrlf=true diff --check
```

Exit code: `0`; output: `<empty>`.

```text
git ls-files --eol .githooks/pre-push scripts/synapse-setup.ps1 crates/synapse-mcp/src/main.rs README.md tests/fixtures/audio/hello_world_5s.wav dashboard/dist/index.html
i/lf    w/lf    attr/text eol=lf       .githooks/pre-push
i/lf    w/lf    attr/text eol=lf       README.md
i/lf    w/lf    attr/text eol=lf       crates/synapse-mcp/src/main.rs
i/-text w/-text attr/-text             dashboard/dist/index.html
i/lf    w/lf    attr/text eol=lf       scripts/synapse-setup.ps1
i/-text w/-text attr/-text             tests/fixtures/audio/hello_world_5s.wav
```

Physical byte readback:

| Path | SHA256 | CRLF | bare LF | bare CR |
| --- | --- | ---: | ---: | ---: |
| `.githooks/pre-push` | `847F0AFBE9011EA6FAD40DD41F8B1ADE5B56A5CF5D12AC25C83ECB0DA205252F` | 0 | 153 | 0 |
| `scripts/synapse-setup.ps1` | `64E57D5B8833DAED018985D4C1868AFB949531635F7091DD945481B7748CC1DE` | 0 | 6922 | 0 |
| `crates/synapse-mcp/src/main.rs` | `CA9E3665FB3FBCAB6E86EA289B405BA1D9ABBAACC757A17E1C7811BFC172A250` | 0 | 2008 | 0 |
| `README.md` | `77D5774994A6CFB2915ACF0381192E3F919AE6889D32D1FC3CF16C1D5021E314` | 0 | 801 | 0 |
| `tests/fixtures/audio/hello_world_5s.wav` | `B811EDEDB0392928DC8673D91A3BE7FC37EC0BEC3E288C97EA928F949D96B6A6` | 0 | 661 | 438 |
| `dashboard/dist/index.html` | `718D63AD664AA4FD374B9F65CFD6C36CD971FEF14765AB02D14634E5998F7BB6` | 11 | 2 | 1 |

The binary WAV and generated dashboard bundle intentionally retain arbitrary
byte patterns while Git reports them as `-text`.

CRLF exception readback for future Windows-only files:

```text
example.cmd: text: set
example.cmd: eol: crlf
example.bat: text: set
example.bat: eol: crlf
example.sln: text: set
example.sln: eol: crlf
```
