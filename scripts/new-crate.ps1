param(
    [Parameter(Mandatory = $true)]
    [string]$Name
)

$ErrorActionPreference = "Stop"

if ($Name -notmatch '^synapse-[a-z0-9-]+$') {
    throw "Name must match ^synapse-[a-z0-9-]+$"
}

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$CrateDir = Join-Path $RepoRoot "crates/$Name"
$Manifest = Join-Path $RepoRoot 'Cargo.toml'

if (Test-Path $CrateDir) {
    throw "Crate already exists: $CrateDir"
}

New-Item -ItemType Directory -Path (Join-Path $CrateDir 'src') -Force | Out-Null

@"
[package]
name = "$Name"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
synapse-core = { version = "0.1.0", path = "../synapse-core" }

[lints]
workspace = true
"@ | Set-Content -Path (Join-Path $CrateDir 'Cargo.toml') -NoNewline

$ShortName = $Name -replace '^synapse-', ''
"// $ShortName - TODO`n" | Set-Content -Path (Join-Path $CrateDir 'src/lib.rs') -NoNewline

$RelativeMember = "    `"crates/$Name`","
$ManifestLines = Get-Content $Manifest
if ($ManifestLines -contains $RelativeMember) {
    throw "Workspace member already exists in Cargo.toml: crates/$Name"
}

$InsertAt = -1
for ($i = 0; $i -lt $ManifestLines.Count; $i++) {
    if ($ManifestLines[$i] -eq ']') {
        $InsertAt = $i
        break
    }
}

if ($InsertAt -lt 0) {
    throw "Could not find workspace members array terminator in Cargo.toml"
}

$Updated = @()
$Updated += $ManifestLines[0..($InsertAt - 1)]
$Updated += $RelativeMember
$Updated += $ManifestLines[$InsertAt..($ManifestLines.Count - 1)]
$Updated | Set-Content $Manifest

Write-Host "Created crates/$Name"
