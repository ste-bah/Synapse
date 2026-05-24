[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [int]$OlderThanDays = 30,

    [string]$Root = ".",

    [switch]$IncludeLegacyFsv
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($OlderThanDays -lt 0) {
    throw "OlderThanDays must be >= 0"
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$cutoff = (Get-Date).AddDays(-$OlderThanDays)

function Assert-UnderRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $rootFullPath = [System.IO.Path]::GetFullPath($script:rootPath)
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    if (-not $fullPath.StartsWith($rootFullPath.TrimEnd('\') + '\', $comparison)) {
        throw "refusing to remove path outside root: $fullPath"
    }
}

function Visit-RunDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.DirectoryInfo]$Directory,

        [Parameter(Mandatory = $true)]
        [string]$Kind
    )

    Assert-UnderRoot -Path $Directory.FullName
    $ageDays = ((Get-Date) - $Directory.LastWriteTime).TotalDays
    $action = "keep"

    if ($Directory.LastWriteTime -lt $script:cutoff) {
        $action = "remove"
        Write-Output (
            "source_of_truth=run_cleanup kind={0} action={1} path=""{2}"" last_write_time=""{3:o}"" age_days={4:F3} cutoff=""{5:o}""" -f
            $Kind,
            $action,
            $Directory.FullName,
            $Directory.LastWriteTime,
            $ageDays,
            $script:cutoff
        )
        if ($PSCmdlet.ShouldProcess($Directory.FullName, "Remove old run directory")) {
            Remove-Item -LiteralPath $Directory.FullName -Recurse -Force
        }
        return
    }

    Write-Output (
        "source_of_truth=run_cleanup kind={0} action={1} path=""{2}"" last_write_time=""{3:o}"" age_days={4:F3} cutoff=""{5:o}""" -f
        $Kind,
        $action,
        $Directory.FullName,
        $Directory.LastWriteTime,
        $ageDays,
        $script:cutoff
    )
}

$runsPath = Join-Path $rootPath ".runs"
if (Test-Path -LiteralPath $runsPath) {
    Get-ChildItem -LiteralPath $runsPath -Directory -Force |
        ForEach-Object { Visit-RunDirectory -Directory $_ -Kind ".runs" }
}

if ($IncludeLegacyFsv) {
    Get-ChildItem -LiteralPath $rootPath -Directory -Force -Filter "fsv-*" |
        ForEach-Object { Visit-RunDirectory -Directory $_ -Kind "legacy_fsv" }
}
