<#
.SYNOPSIS
  Windows-side Synapse setup: build/install the daemon binary, deploy bundled
  profiles, generate the bearer token, register the auto-start HTTP daemon, and
  (optionally) wire the Windows-side MCP clients. Idempotent and fail-loud.

.DESCRIPTION
  Synapse has exactly ONE controlling body: the Windows-native synapse-mcp.exe
  HTTP daemon. It is the only process that can do real Win32 SendInput / UI
  Automation / WGC-DXGI capture, and it controls BOTH Windows programs (native
  windows) and WSL programs (WSLg GUI apps render as real Windows windows;
  act_run_shell / act_launch reach WSL CLIs via wsl.exe). Every MCP client — on
  Windows or in WSL — connects to this one daemon.

  This script makes that body exist and run, then points the Windows-side
  clients at it. The WSL-side entry (scripts/synapse-install.sh) calls this same
  script through interop and then wires the WSL-side clients.

  Robustness decisions baked in here (learned the hard way):
    * Build from the LOCAL source path (cd into -SourceDir). Building over a
      \\wsl.localhost / pushd-mapped drive bakes transient Z:\ paths into the
      binary (CARGO_MANIFEST_DIR) and intermittently fails cargo's dep-info
      step. -SourceDir must be a real local path.
    * Deploy the bundled profiles NEXT TO the installed exe so the daemon's
      executable-relative profile lookup always resolves, and ALSO pass
      --profile-dir explicitly. A compile-time CARGO_MANIFEST_DIR profile path
      never exists on an installed host.
    * Use a persistent CARGO_TARGET_DIR so re-installs are incremental, not a
      ~25-minute RocksDB rebuild every time.

  Nothing here silently falls back: every prerequisite is checked and throws a
  clear error naming exactly what failed and how to fix it.

.PARAMETER SourceDir
  Path to a LOCAL synapse source checkout to build from. Required unless
  -SkipBuild is set. Must be on a real local drive (not \\wsl.localhost or a
  pushd-mapped UNC drive).

.PARAMETER SkipBuild
  Do not build; require an already-installed synapse-mcp.exe at -ExePath.

.PARAMETER BuildTimeoutMinutes
  Maximum time to allow the release build to run. The build process tree is
  launched inside a Windows Job Object with kill-on-close, so Cargo/rustc
  children cannot survive if this setup process exits or is killed.

.PARAMETER ForceRestart
  Permit setup/remove to stop the shared daemon even when active HTTP MCP
  sessions, live client TCP connections, or bridge children are present. Without
  this explicit maintenance flag, setup fails closed instead of interrupting
  another agent.

.PARAMETER Bind
  Loopback address the daemon binds. Default 127.0.0.1:7700.

.PARAMETER MaintenanceLockPath
  File-lock Source of Truth that serializes setup/remove across multiple
  agents. The file contents name the owning PID and cleanup policy; the held
  FileStream is the actual lock and is released by Windows when setup exits.

.PARAMETER WireClients
  Wire the Windows-side MCP clients (Claude Code and Codex via HTTP, Claude
  Desktop via the connect bridge). Default $true.

.PARAMETER Remove
  Uninstall: stop + unregister the scheduled task. Leaves the DB, token, and
  binary in place unless -Purge is also given.

.PARAMETER Purge
  With -Remove, also delete the daemon DB, deployed profiles, and token.
#>
[CmdletBinding()]
param(
    [string]$SourceDir,
    [switch]$SkipBuild,
    [string]$Bind        = '127.0.0.1:7700',
    [string]$ExePath     = "$env:USERPROFILE\.cargo\bin\synapse-mcp.exe",
    [string]$CargoTarget = "$env:LOCALAPPDATA\synapse\build-target",
    [string]$DbPath      = "$env:LOCALAPPDATA\synapse\db-daemon",
    [string]$ProfilesDir = "$env:USERPROFILE\.cargo\bin\profiles",
    [string]$LogDir      = "$env:LOCALAPPDATA\synapse\logs",
    [string]$TokenPath   = "$env:APPDATA\synapse\token.txt",
    [string]$TaskName    = 'SynapseMcpDaemon',
    [string]$MaintenanceLockPath = "$env:LOCALAPPDATA\synapse\setup-maintenance.lock.json",
    [ValidateRange(1, 1440)][int]$BuildTimeoutMinutes = 90,
    [switch]$ForceRestart,
    [switch]$SkipClientWiring,
    [switch]$Remove,
    [switch]$Purge
)

$ErrorActionPreference = 'Stop'
function Info($m)  { Write-Host "[synapse-setup] $m" }
function Step($m)  { Write-Host "`n=== $m ===" -ForegroundColor Cyan }
function Die($m)   { throw "[synapse-setup] FATAL: $m" }

$processTokenAtStart = $env:SYNAPSE_BEARER_TOKEN
$script:SynapseSetupMaintenanceLockStream = $null

function Get-ProcessLineage {
    param([int]$StartPid = $PID)
    $lineage = @()
    $seen = @{}
    $current = $StartPid
    while ($current -and -not $seen.ContainsKey($current)) {
        $seen[$current] = $true
        $p = Get-CimInstance Win32_Process -Filter "ProcessId=$current" -ErrorAction SilentlyContinue
        if (-not $p) { break }
        $lineage += $p
        $current = [int]$p.ParentProcessId
    }
    return $lineage
}

function Read-SynapseSetupMaintenanceLockOwner {
    param([Parameter(Mandatory=$true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        return '<missing>'
    }
    try {
        $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
        try {
            $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8, $true, 4096, $true)
            try {
                $text = $reader.ReadToEnd().Trim()
                if ([string]::IsNullOrWhiteSpace($text)) { return '<empty>' }
                return ($text -replace '\s+', ' ')
            } finally {
                $reader.Dispose()
            }
        } finally {
            $stream.Dispose()
        }
    } catch {
        return "<unreadable error=$($_.Exception.Message)>"
    }
}

function Acquire-SynapseSetupMaintenanceLock {
    param(
        [Parameter(Mandatory=$true)][string]$Path,
        [Parameter(Mandatory=$true)][string]$Reason
    )

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    try {
        $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::OpenOrCreate, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::Read)
    } catch [System.IO.IOException] {
        $owner = Read-SynapseSetupMaintenanceLockOwner -Path $Path
        Die "SYNAPSE_SETUP_MAINTENANCE_LOCK_HELD reason=$Reason path=$Path owner=$owner remediation=another setup/remove process owns the maintenance lock; wait for that setup process to exit or inspect the named PID. Do not close terminal windows or broad shell processes to clear this condition."
    } catch {
        Die "SYNAPSE_SETUP_MAINTENANCE_LOCK_OPEN_FAILED reason=$Reason path=$Path error=$($_.Exception.Message) remediation=repair permissions on the synapse local appdata directory"
    }

    $script:SynapseSetupMaintenanceLockStream = $stream
    $self = Get-CimInstance Win32_Process -Filter "ProcessId=$PID" -ErrorAction SilentlyContinue
    $lineageText = (Get-ProcessLineage | ForEach-Object { "{0}:{1}" -f $_.ProcessId, $_.Name }) -join ' <- '
    $owner = [ordered]@{
        schema = 'synapse_setup_maintenance_lock/v1'
        state = 'held'
        reason = $Reason
        pid = $PID
        parent_pid = $self.ParentProcessId
        process_name = $self.Name
        command_line = $self.CommandLine
        source_dir = $SourceDir
        bind = $Bind
        started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        lineage = $lineageText
        cleanup_policy = 'never close terminal windows globally; only exact process IDs spawned by this setup operation or verified synapse-mcp targets may be stopped'
    }
    $json = $owner | ConvertTo-Json -Depth 6
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json + "`n")
    $stream.SetLength(0)
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush($true)
    Info "Maintenance lock acquired reason=$Reason path=$Path pid=$PID"
}

function Quote-WindowsCommandArgument {
    param([Parameter(Mandatory=$true)][string]$Value)
    if ($Value.Length -eq 0) { return '""' }
    if ($Value -notmatch '[\s"]') { return $Value }
    $escaped = $Value -replace '(\\*)"', '$1$1\"'
    $escaped = $escaped -replace '(\\+)$', '$1$1'
    return '"' + $escaped + '"'
}

function Quote-VbsString {
    param([Parameter(Mandatory=$true)][string]$Value)
    return '"' + ($Value -replace '"', '""') + '"'
}

function Vbs-Literal {
    param([Parameter(Mandatory=$true)][string]$Value)
    return '"' + ($Value -replace '"', '""') + '"'
}

function New-HiddenDaemonLauncher {
    param(
        [Parameter(Mandatory=$true)][string]$OutputPath,
        [Parameter(Mandatory=$true)][string]$ExePath,
        [Parameter(Mandatory=$true)][string]$Bind,
        [Parameter(Mandatory=$true)][string]$DbPath,
        [Parameter(Mandatory=$true)][string]$ProfilesDir,
        [Parameter(Mandatory=$true)][string]$LogDir,
        [Parameter(Mandatory=$true)][string]$TokenPath
    )

    $daemonLogDir = $LogDir
    $launcherLog = Join-Path $LogDir 'daemon-launcher.log'
    $daemonCommand = @(
        (Quote-WindowsCommandArgument $ExePath),
        '--mode', 'http',
        '--bind', (Quote-WindowsCommandArgument $Bind),
        '--db', (Quote-WindowsCommandArgument $DbPath),
        '--profile-dir', (Quote-WindowsCommandArgument $ProfilesDir),
        '--log-level', 'info'
    ) -join ' '

    @"
Option Explicit
Dim shell, fso, env, tokenPath, launcherLog, daemonLogDir, daemonCommand
Dim tokenFile, token, exitCode

Set shell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
tokenPath = $(Vbs-Literal $TokenPath)
launcherLog = $(Vbs-Literal $launcherLog)
daemonLogDir = $(Vbs-Literal $daemonLogDir)
daemonCommand = $(Vbs-Literal $daemonCommand)

Sub LogLine(message)
  Dim logFile
  Set logFile = fso.OpenTextFile(launcherLog, 8, True)
  logFile.WriteLine Now & " " & message
  logFile.Close
End Sub

On Error Resume Next
If Not fso.FolderExists(daemonLogDir) Then
  fso.CreateFolder daemonLogDir
End If
If Err.Number <> 0 Then
  WScript.Quit 1
End If
On Error GoTo 0

If Not fso.FileExists(tokenPath) Then
  LogLine "SYNAPSE_DAEMON_TOKEN_MISSING path=" & tokenPath
  WScript.Quit 1
End If

On Error Resume Next
Set tokenFile = fso.OpenTextFile(tokenPath, 1, False)
If Err.Number <> 0 Then
  LogLine "SYNAPSE_DAEMON_TOKEN_READ_FAILED path=" & tokenPath & " err_number=" & Err.Number & " err_description=" & Err.Description
  WScript.Quit 1
End If
If tokenFile.AtEndOfStream Then
  token = ""
Else
  token = Trim(tokenFile.ReadAll)
End If
If Err.Number <> 0 Then
  LogLine "SYNAPSE_DAEMON_TOKEN_READ_FAILED path=" & tokenPath & " err_number=" & Err.Number & " err_description=" & Err.Description
  tokenFile.Close
  WScript.Quit 1
End If
tokenFile.Close
On Error GoTo 0

If Len(token) < 16 Then
  LogLine "SYNAPSE_DAEMON_TOKEN_INVALID path=" & tokenPath & " length=" & Len(token)
  WScript.Quit 1
End If

Set env = shell.Environment("PROCESS")
env("SYNAPSE_BEARER_TOKEN") = token
env("SYNAPSE_LOG_DIR") = daemonLogDir

LogLine "SYNAPSE_DAEMON_LAUNCH_START command=" & daemonCommand
exitCode = shell.Run(daemonCommand, 0, True)
LogLine "SYNAPSE_DAEMON_EXIT exit_code=" & exitCode
WScript.Quit exitCode
"@ | Set-Content -Path $OutputPath -Encoding ascii
}

function Ensure-SynapseSetupProcessJobType {
    if ('SynapseSetup.ProcessJob' -as [type]) { return }

    Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace SynapseSetup
{
    public static class ProcessJob
    {
        private const uint CREATE_SUSPENDED = 0x00000004;
        private const uint CREATE_NO_WINDOW = 0x08000000;
        private const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        private const int JobObjectExtendedLimitInformation = 9;
        private const uint WAIT_OBJECT_0 = 0x00000000;
        private const uint WAIT_TIMEOUT = 0x00000102;
        private const uint WAIT_FAILED = 0xffffffff;
        private const uint EXIT_TIMEOUT = 124;
        private const uint EXIT_ASSIGN_FAILED = 125;
        private const uint EXIT_RESUME_FAILED = 126;
        private const uint INFINITE = 0xffffffff;

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS
        {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct STARTUPINFO
        {
            public uint cb;
            public string lpReserved;
            public string lpDesktop;
            public string lpTitle;
            public uint dwX;
            public uint dwY;
            public uint dwXSize;
            public uint dwYSize;
            public uint dwXCountChars;
            public uint dwYCountChars;
            public uint dwFillAttribute;
            public uint dwFlags;
            public ushort wShowWindow;
            public ushort cbReserved2;
            public IntPtr lpReserved2;
            public IntPtr hStdInput;
            public IntPtr hStdOutput;
            public IntPtr hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct PROCESS_INFORMATION
        {
            public IntPtr hProcess;
            public IntPtr hThread;
            public uint dwProcessId;
            public uint dwThreadId;
        }

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        private static extern IntPtr CreateJobObject(IntPtr lpJobAttributes, string lpName);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(
            IntPtr hJob,
            int jobObjectInfoClass,
            IntPtr lpJobObjectInfo,
            uint cbJobObjectInfoLength);

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        private static extern bool CreateProcess(
            string lpApplicationName,
            StringBuilder lpCommandLine,
            IntPtr lpProcessAttributes,
            IntPtr lpThreadAttributes,
            bool bInheritHandles,
            uint dwCreationFlags,
            IntPtr lpEnvironment,
            string lpCurrentDirectory,
            ref STARTUPINFO lpStartupInfo,
            out PROCESS_INFORMATION lpProcessInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr hJob, IntPtr hProcess);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr hThread);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr hHandle, uint dwMilliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetExitCodeProcess(IntPtr hProcess, out uint lpExitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(IntPtr hJob, uint uExitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr hProcess, uint uExitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr hObject);

        public static int Run(
            string applicationName,
            string commandLine,
            string workingDirectory,
            uint timeoutMilliseconds,
            out string failure)
        {
            failure = "";
            IntPtr job = IntPtr.Zero;
            IntPtr limitPointer = IntPtr.Zero;
            PROCESS_INFORMATION processInfo = new PROCESS_INFORMATION();
            try
            {
                job = CreateJobObject(IntPtr.Zero, null);
                if (job == IntPtr.Zero)
                {
                    failure = "PROCESS_JOB_CREATE_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    return 127;
                }

                JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                int limitSize = Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION));
                limitPointer = Marshal.AllocHGlobal(limitSize);
                Marshal.StructureToPtr(limits, limitPointer, false);
                if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, limitPointer, (uint)limitSize))
                {
                    failure = "PROCESS_JOB_LIMIT_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    return 127;
                }

                STARTUPINFO startupInfo = new STARTUPINFO();
                startupInfo.cb = (uint)Marshal.SizeOf(typeof(STARTUPINFO));
                StringBuilder mutableCommandLine = new StringBuilder(commandLine);
                bool created = CreateProcess(
                    applicationName,
                    mutableCommandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    false,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW,
                    IntPtr.Zero,
                    workingDirectory,
                    ref startupInfo,
                    out processInfo);
                if (!created)
                {
                    failure = "PROCESS_JOB_CREATE_PROCESS_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    return 127;
                }

                if (!AssignProcessToJobObject(job, processInfo.hProcess))
                {
                    failure = "PROCESS_JOB_ASSIGN_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    TerminateProcess(processInfo.hProcess, EXIT_ASSIGN_FAILED);
                    return (int)EXIT_ASSIGN_FAILED;
                }

                if (ResumeThread(processInfo.hThread) == 0xffffffff)
                {
                    failure = "PROCESS_JOB_RESUME_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    TerminateJobObject(job, EXIT_RESUME_FAILED);
                    return (int)EXIT_RESUME_FAILED;
                }

                uint wait = WaitForSingleObject(
                    processInfo.hProcess,
                    timeoutMilliseconds == 0 ? INFINITE : timeoutMilliseconds);
                if (wait == WAIT_TIMEOUT)
                {
                    TerminateJobObject(job, EXIT_TIMEOUT);
                    WaitForSingleObject(processInfo.hProcess, 15000);
                    failure = "PROCESS_JOB_TIMEOUT: child process tree exceeded timeout_ms=" + timeoutMilliseconds;
                    return (int)EXIT_TIMEOUT;
                }
                if (wait == WAIT_FAILED)
                {
                    failure = "PROCESS_JOB_WAIT_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    TerminateJobObject(job, 127);
                    return 127;
                }
                if (wait != WAIT_OBJECT_0)
                {
                    failure = "PROCESS_JOB_WAIT_UNEXPECTED: wait_result=" + wait;
                    TerminateJobObject(job, 127);
                    return 127;
                }

                uint exitCode;
                if (!GetExitCodeProcess(processInfo.hProcess, out exitCode))
                {
                    failure = "PROCESS_JOB_EXIT_CODE_FAILED: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    return 127;
                }
                return unchecked((int)exitCode);
            }
            finally
            {
                if (limitPointer != IntPtr.Zero)
                {
                    Marshal.FreeHGlobal(limitPointer);
                }
                if (processInfo.hThread != IntPtr.Zero)
                {
                    CloseHandle(processInfo.hThread);
                }
                if (processInfo.hProcess != IntPtr.Zero)
                {
                    CloseHandle(processInfo.hProcess);
                }
                if (job != IntPtr.Zero)
                {
                    CloseHandle(job);
                }
            }
        }
    }
}
'@ | Out-Null
}

function Invoke-SynapseProcessInKillOnCloseJob {
    param(
        [Parameter(Mandatory=$true)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory=$true)][string]$WorkingDirectory,
        [Parameter(Mandatory=$true)][int]$TimeoutMinutes,
        [string]$LogPath
    )

    Ensure-SynapseSetupProcessJobType
    if (-not (Test-Path $FilePath)) { Die "Process job target missing: $FilePath" }
    if (-not (Test-Path $WorkingDirectory)) { Die "Process job working directory missing: $WorkingDirectory" }

    $argumentText = (($ArgumentList | ForEach-Object { Quote-WindowsCommandArgument $_ }) -join ' ').Trim()
    $targetCommand = (Quote-WindowsCommandArgument $FilePath)
    if (-not [string]::IsNullOrWhiteSpace($argumentText)) {
        $targetCommand = "$targetCommand $argumentText"
    }

    $applicationPath = $FilePath
    $commandLine = $targetCommand
    if (-not [string]::IsNullOrWhiteSpace($LogPath)) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LogPath) | Out-Null
        if (Test-Path $LogPath) { Remove-Item $LogPath -Force }
        $cmdPath = Join-Path $env:SystemRoot 'System32\cmd.exe'
        $redirectCommand = "$targetCommand > $(Quote-WindowsCommandArgument $LogPath) 2>&1"
        $applicationPath = $cmdPath
        $commandLine = "$(Quote-WindowsCommandArgument $cmdPath) /d /s /c `"$redirectCommand`""
    }

    $timeoutMilliseconds = [uint32]([math]::Min([int64]$TimeoutMinutes * 60 * 1000, [uint32]::MaxValue))
    $failure = ''
    $exitCode = [SynapseSetup.ProcessJob]::Run(
        $applicationPath,
        $commandLine,
        $WorkingDirectory,
        $timeoutMilliseconds,
        [ref]$failure)
    if (-not [string]::IsNullOrWhiteSpace($failure)) {
        $tail = ''
        if ($LogPath -and (Test-Path $LogPath)) {
            $tail = (Get-Content -Path $LogPath -Tail 40 -ErrorAction SilentlyContinue) -join "`n"
        }
        Die "PROCESS_JOB_FAILED command=$targetCommand exit=$exitCode reason=$failure log=$LogPath tail=`n$tail"
    }
    return $exitCode
}

function Install-CodexSynapseTokenLoader {
    param(
        [Parameter(Mandatory=$true)][string]$CodexCommandPath,
        [Parameter(Mandatory=$true)][string]$TokenPath
    )

    $npmDir = Split-Path -Parent $CodexCommandPath
    if (-not $npmDir -or -not (Test-Path $npmDir)) {
        Die "Cannot resolve Codex launcher directory from '$CodexCommandPath'."
    }

    $ps1Path = Join-Path $npmDir 'codex.ps1'
    $cmdPath = Join-Path $npmDir 'codex.cmd'
    $shPath = Join-Path $npmDir 'codex'

    if (Test-Path $ps1Path) {
        $ps1 = @'
#!/usr/bin/env pwsh
$basedir=Split-Path $MyInvocation.MyCommand.Definition -Parent

# Synapse MCP token loader: begin
$synapseConfigPath = Join-Path $env:USERPROFILE '.codex\config.toml'
$synapseTokenPath = Join-Path $env:APPDATA 'synapse\token.txt'
$synapseHasConfig = $false
if (Test-Path $synapseConfigPath) {
  try {
    $synapseHasConfig = ((Get-Content -Raw $synapseConfigPath) -match '(?m)^\[mcp_servers\.synapse\]')
  } catch {
    Write-Error "SYNAPSE_CODEX_CONFIG_UNREADABLE path=$synapseConfigPath remediation=repair Codex config permissions or rerun scripts\synapse-setup.ps1"
    exit 1
  }
}
if ($synapseHasConfig) {
  if (-not (Test-Path $synapseTokenPath)) {
    Write-Error "SYNAPSE_CODEX_TOKEN_MISSING path=$synapseTokenPath remediation=run scripts\synapse-setup.ps1 to generate the bearer token"
    exit 1
  }
  $synapseTokenRaw = Get-Content -Raw $synapseTokenPath
  $synapseToken = if ($null -eq $synapseTokenRaw) { '' } else { $synapseTokenRaw.Trim() }
  if ([string]::IsNullOrWhiteSpace($synapseToken)) {
    Write-Error "SYNAPSE_CODEX_TOKEN_EMPTY path=$synapseTokenPath remediation=delete the empty token and rerun scripts\synapse-setup.ps1"
    exit 1
  }
  if ($env:SYNAPSE_BEARER_TOKEN -ne $synapseToken) {
    $env:SYNAPSE_BEARER_TOKEN = $synapseToken
  }
}
Remove-Variable synapseConfigPath,synapseTokenPath,synapseHasConfig -ErrorAction SilentlyContinue
Remove-Variable synapseTokenRaw,synapseToken -ErrorAction SilentlyContinue
# Synapse MCP token loader: end

$exe=""
if ($PSVersionTable.PSVersion -lt "6.0" -or $IsWindows) {
  # Fix case when both the Windows and Linux builds of Node
  # are installed in the same directory
  $exe=".exe"
}
$ret=0
if (Test-Path "$basedir/node$exe") {
  # Support pipeline input
  if ($MyInvocation.ExpectingInput) {
    $input | & "$basedir/node$exe"  "$basedir/node_modules/@openai/codex/bin/codex.js" $args
  } else {
    & "$basedir/node$exe"  "$basedir/node_modules/@openai/codex/bin/codex.js" $args
  }
  $ret=$LASTEXITCODE
} else {
  # Support pipeline input
  if ($MyInvocation.ExpectingInput) {
    $input | & "node$exe"  "$basedir/node_modules/@openai/codex/bin/codex.js" $args
  } else {
    & "node$exe"  "$basedir/node_modules/@openai/codex/bin/codex.js" $args
  }
  $ret=$LASTEXITCODE
}
exit $ret
'@
        Copy-Item $ps1Path "$ps1Path.synapse-bak" -Force
        Set-Content -Path $ps1Path -Value $ps1 -Encoding utf8
        Info "Installed Synapse token loader in Codex PowerShell launcher: $ps1Path"
    } else {
        Info "WARN: Codex PowerShell launcher not found at $ps1Path; cannot install ps1 token loader."
    }

    if (Test-Path $cmdPath) {
        $cmd = @'
@ECHO off
GOTO start
:find_dp0
SET dp0=%~dp0
EXIT /b
:start
SETLOCAL
CALL :find_dp0

REM Synapse MCP token loader: begin
SET "_synapse_cfg=%USERPROFILE%\.codex\config.toml"
SET "_synapse_tok=%APPDATA%\synapse\token.txt"
SET "_synapse_has_cfg="
IF EXIST "%_synapse_cfg%" (
  %SystemRoot%\System32\findstr.exe /R /C:"^\[mcp_servers\.synapse\]" "%_synapse_cfg%" >NUL 2>NUL
  IF NOT ERRORLEVEL 1 SET "_synapse_has_cfg=1"
)
IF DEFINED _synapse_has_cfg (
  IF NOT EXIST "%_synapse_tok%" (
    ECHO SYNAPSE_CODEX_TOKEN_MISSING path=%_synapse_tok% remediation=run scripts\synapse-setup.ps1 to generate the bearer token 1>&2
    EXIT /B 1
  )
  SET /P _synapse_file_token=<"%_synapse_tok%"
  IF NOT DEFINED _synapse_file_token (
    ECHO SYNAPSE_CODEX_TOKEN_EMPTY path=%_synapse_tok% remediation=delete the empty token and rerun scripts\synapse-setup.ps1 1>&2
    EXIT /B 1
  )
  IF NOT "%SYNAPSE_BEARER_TOKEN%"=="%_synapse_file_token%" SET "SYNAPSE_BEARER_TOKEN=%_synapse_file_token%"
)
SET "_synapse_cfg="
SET "_synapse_tok="
SET "_synapse_has_cfg="
SET "_synapse_file_token="
REM Synapse MCP token loader: end

IF EXIST "%dp0%\node.exe" (
  SET "_prog=%dp0%\node.exe"
) ELSE (
  SET "_prog=node"
  SET PATHEXT=%PATHEXT:;.JS;=;%
)

endLocal & SET "SYNAPSE_BEARER_TOKEN=%SYNAPSE_BEARER_TOKEN%" & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\node_modules\@openai\codex\bin\codex.js" %*
'@
        Copy-Item $cmdPath "$cmdPath.synapse-bak" -Force
        Set-Content -Path $cmdPath -Value $cmd -Encoding ascii
        Info "Installed Synapse token loader in Codex CMD launcher: $cmdPath"
    } else {
        Info "WARN: Codex CMD launcher not found at $cmdPath; cannot install cmd token loader."
    }

    if (Test-Path $shPath) {
        $sh = @'
#!/bin/sh
basedir=$(dirname "$(echo "$0" | sed -e 's,\\,/,g')")

# Synapse MCP token loader: begin
synapse_cfg="$USERPROFILE/.codex/config.toml"
synapse_tok="$APPDATA/synapse/token.txt"
case `uname` in
    *CYGWIN*|*MINGW*|*MSYS*)
        if command -v cygpath > /dev/null 2>&1; then
            synapse_cfg=$(cygpath -u "$synapse_cfg")
            synapse_tok=$(cygpath -u "$synapse_tok")
        fi
    ;;
esac
if [ -f "$synapse_cfg" ] && grep -Eq '^\[mcp_servers\.synapse\]' "$synapse_cfg"; then
    if [ ! -r "$synapse_tok" ]; then
        printf '%s\n' "SYNAPSE_CODEX_TOKEN_MISSING path=$synapse_tok remediation=run scripts/synapse-setup.ps1 to generate the bearer token" >&2
        exit 1
    fi
    synapse_file_token=$(tr -d '\r\n' < "$synapse_tok")
    if [ -z "$synapse_file_token" ]; then
        printf '%s\n' "SYNAPSE_CODEX_TOKEN_EMPTY path=$synapse_tok remediation=delete the empty token and rerun scripts/synapse-setup.ps1" >&2
        exit 1
    fi
    if [ "${SYNAPSE_BEARER_TOKEN:-}" != "$synapse_file_token" ]; then
        SYNAPSE_BEARER_TOKEN="$synapse_file_token"
        export SYNAPSE_BEARER_TOKEN
    fi
fi
unset synapse_cfg synapse_tok synapse_file_token
# Synapse MCP token loader: end

case `uname` in
    *CYGWIN*|*MINGW*|*MSYS*)
        if command -v cygpath > /dev/null 2>&1; then
            basedir=`cygpath -w "$basedir"`
        fi
    ;;
esac

if [ -x "$basedir/node" ]; then
  exec "$basedir/node"  "$basedir/node_modules/@openai/codex/bin/codex.js" "$@"
else
  exec node  "$basedir/node_modules/@openai/codex/bin/codex.js" "$@"
fi
'@
        Copy-Item $shPath "$shPath.synapse-bak" -Force
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($shPath, ($sh -replace "`r?`n", "`n"), $utf8NoBom)
        Info "Installed Synapse token loader in Codex shell launcher: $shPath"
    } else {
        Info "WARN: Codex shell launcher not found at $shPath; cannot install shell token loader."
    }

    $loaderTokenRaw = if (Test-Path $TokenPath) { Get-Content -Raw $TokenPath } else { $null }
    $loaderToken = if ($null -eq $loaderTokenRaw) { '' } else { $loaderTokenRaw.Trim() }
    if ((Test-Path $TokenPath) -and [string]::IsNullOrWhiteSpace($loaderToken)) {
        Die "Installed Codex token loaders, but token at $TokenPath is empty."
    }
}

function Get-SynapseMcpProcessSnapshot {
    @(Get-CimInstance Win32_Process -Filter "Name='synapse-mcp.exe'" -ErrorAction SilentlyContinue |
        Sort-Object ProcessId |
        Select-Object ProcessId, ParentProcessId, Name, CommandLine)
}

function Format-SynapseMcpProcessSnapshot {
    param([object[]]$Snapshot)
    if (-not $Snapshot -or $Snapshot.Count -eq 0) {
        return '<none>'
    }
    return (($Snapshot | ForEach-Object {
        "pid=$($_.ProcessId) ppid=$($_.ParentProcessId) cmd=$($_.CommandLine)"
    }) -join "`n")
}

function Get-SynapseBindEndpoint {
    param([Parameter(Mandatory=$true)][string]$Bind)

    $lastColon = $Bind.LastIndexOf(':')
    if ($lastColon -lt 1 -or $lastColon -eq ($Bind.Length - 1)) {
        Die "SYNAPSE_BIND_PARSE_FAILED bind=$Bind remediation=use host:port, for example 127.0.0.1:7700"
    }

    $address = $Bind.Substring(0, $lastColon)
    $portText = $Bind.Substring($lastColon + 1)
    $port = 0
    if (-not [int]::TryParse($portText, [ref]$port) -or $port -lt 1 -or $port -gt 65535) {
        Die "SYNAPSE_BIND_PARSE_FAILED bind=$Bind port=$portText remediation=use a TCP port from 1 through 65535"
    }

    [pscustomobject]@{ Address = $address; Port = $port }
}

function Get-SynapseTcpClientSnapshot {
    param([Parameter(Mandatory=$true)][string]$Bind)

    $endpoint = Get-SynapseBindEndpoint -Bind $Bind
    $allTcp = @(Get-NetTCPConnection -ErrorAction SilentlyContinue)
    $serverConnections = @($allTcp |
        Where-Object {
            $_.LocalAddress -eq $endpoint.Address -and
            $_.LocalPort -eq $endpoint.Port -and
            "$($_.State)" -ne 'Listen'
        } |
        Sort-Object LocalPort, RemotePort, OwningProcess)

    foreach ($connection in $serverConnections) {
        $peer = @($allTcp | Where-Object {
            $_.LocalAddress -eq $connection.RemoteAddress -and
            $_.LocalPort -eq $connection.RemotePort -and
            $_.RemoteAddress -eq $connection.LocalAddress -and
            $_.RemotePort -eq $connection.LocalPort
        } | Select-Object -First 1)
        $peerOwnerPid = if ($peer.Count -gt 0) { [int]$peer[0].OwningProcess } else { 0 }
        $peerOwner = if ($peerOwnerPid -gt 0) { Get-Process -Id $peerOwnerPid -ErrorAction SilentlyContinue } else { $null }
        $peerOwnerLine = if ($peerOwnerPid -gt 0) {
            (Get-CimInstance Win32_Process -Filter "ProcessId=$peerOwnerPid" -ErrorAction SilentlyContinue).CommandLine
        } else {
            $null
        }
        [pscustomobject]@{
            State = $connection.State
            LocalAddress = $connection.LocalAddress
            LocalPort = $connection.LocalPort
            RemoteAddress = $connection.RemoteAddress
            RemotePort = $connection.RemotePort
            OwningProcess = $connection.OwningProcess
            OwnerName = (Get-Process -Id $connection.OwningProcess -ErrorAction SilentlyContinue).ProcessName
            OwnerCommandLine = (Get-CimInstance Win32_Process -Filter "ProcessId=$($connection.OwningProcess)" -ErrorAction SilentlyContinue).CommandLine
            PeerOwningProcess = $peerOwnerPid
            PeerOwnerName = $peerOwner.ProcessName
            PeerOwnerCommandLine = $peerOwnerLine
            HasLivePeer = ($peerOwnerPid -gt 0)
        }
    }
}

function Format-SynapseTcpClientSnapshot {
    param([object[]]$Snapshot)
    if (-not $Snapshot -or $Snapshot.Count -eq 0) {
        return '<none>'
    }
    return (($Snapshot | ForEach-Object {
        "state=$($_.State) local=$($_.LocalAddress):$($_.LocalPort) remote=$($_.RemoteAddress):$($_.RemotePort) owner_pid=$($_.OwningProcess) owner=$($_.OwnerName) peer_pid=$($_.PeerOwningProcess) peer=$($_.PeerOwnerName) has_live_peer=$($_.HasLivePeer) peer_cmd=$($_.PeerOwnerCommandLine)"
    }) -join "`n")
}

function Read-SynapseSetupTokenForRestartGuard {
    param([Parameter(Mandatory=$true)][string]$TokenPath)

    if (-not (Test-Path -LiteralPath $TokenPath)) {
        return [pscustomobject]@{ Ok = $false; Code = 'SYNAPSE_RESTART_GUARD_TOKEN_MISSING'; Token = $null; Detail = "path=$TokenPath" }
    }

    try {
        $raw = Get-Content -Raw -LiteralPath $TokenPath
        $token = if ($null -eq $raw) { '' } else { $raw.Trim() }
    } catch {
        return [pscustomobject]@{ Ok = $false; Code = 'SYNAPSE_RESTART_GUARD_TOKEN_READ_FAILED'; Token = $null; Detail = "path=$TokenPath error=$($_.Exception.Message)" }
    }

    if ($token.Length -lt 16) {
        return [pscustomobject]@{ Ok = $false; Code = 'SYNAPSE_RESTART_GUARD_TOKEN_INVALID'; Token = $null; Detail = "path=$TokenPath length=$($token.Length)" }
    }

    [pscustomobject]@{ Ok = $true; Code = 'OK'; Token = $token; Detail = "path=$TokenPath length=$($token.Length)" }
}

function Read-SynapseHealthForRestartGuard {
    param(
        [Parameter(Mandatory=$true)][string]$Bind,
        [Parameter(Mandatory=$true)][string]$Token
    )

    try {
        $health = Invoke-RestMethod -Uri "http://$Bind/health" -Headers @{ Authorization = "Bearer $Token" } -TimeoutSec 4
        [pscustomobject]@{ Ok = $true; Health = $health; Error = $null }
    } catch {
        [pscustomobject]@{ Ok = $false; Health = $null; Error = $_.Exception.Message }
    }
}

function Get-SynapseActiveSessionCount {
    param([Parameter(Mandatory=$true)]$Health)

    $value = $Health.subsystems.http.active_sessions
    if ($null -eq $value) {
        return $null
    }

    try {
        return [int]$value
    } catch {
        return $null
    }
}

function Assert-SynapseRestartAllowed {
    param(
        [Parameter(Mandatory=$true)][string]$Reason,
        [Parameter(Mandatory=$true)][string]$Bind,
        [Parameter(Mandatory=$true)][string]$TokenPath,
        [switch]$ForceRestart
    )

    $processes = Get-SynapseMcpProcessSnapshot
    if ($processes.Count -eq 0) {
        Info "Synapse restart guard reason=$Reason existing_process_count=0 verdict=clear"
        return
    }

    $null = Get-SynapseBindEndpoint -Bind $Bind
    $nonHttpProcesses = @($processes | Where-Object { $_.CommandLine -notmatch '(?i)--mode\s+http' })
    $tokenRead = Read-SynapseSetupTokenForRestartGuard -TokenPath $TokenPath
    if (-not $tokenRead.Ok) {
        $message = "$($tokenRead.Code) reason=$Reason process_count=$($processes.Count) $($tokenRead.Detail) remediation=do not restart blindly while the daemon may have clients; repair token state or rerun with -ForceRestart after coordinating a maintenance window"
        if ($ForceRestart) {
            Info "FORCE_RESTART: $message"
        } else {
            Die $message
        }
    }

    $activeSessions = $null
    $healthRead = $null
    if ($tokenRead.Ok) {
        $healthRead = Read-SynapseHealthForRestartGuard -Bind $Bind -Token $tokenRead.Token
        if (-not $healthRead.Ok) {
            $message = "SYNAPSE_RESTART_GUARD_HEALTH_UNREADABLE reason=$Reason bind=$Bind error=$($healthRead.Error) remediation=do not restart blindly; repair the daemon/token or rerun with -ForceRestart after coordinating a maintenance window"
            if ($ForceRestart) {
                Info "FORCE_RESTART: $message"
            } else {
                Die $message
            }
        }
    }

    if ($tokenRead.Ok -and $healthRead.Ok) {
        $activeSessions = Get-SynapseActiveSessionCount -Health $healthRead.Health
        if ($null -eq $activeSessions) {
            $message = "SYNAPSE_RESTART_GUARD_ACTIVE_SESSIONS_UNREADABLE reason=$Reason bind=$Bind remediation=health did not expose subsystems.http.active_sessions; do not restart blindly"
            if ($ForceRestart) {
                Info "FORCE_RESTART: $message"
            } else {
                Die $message
            }
        }
    }

    $tcpConnections = @(Get-SynapseTcpClientSnapshot -Bind $Bind)
    $tcpClients = @($tcpConnections | Where-Object { $_.HasLivePeer })
    $staleTcpConnections = @($tcpConnections | Where-Object { -not $_.HasLivePeer })
    $blockers = @()
    if ($nonHttpProcesses.Count -gt 0) { $blockers += "non_http_synapse_processes=$($nonHttpProcesses.Count)" }
    if ($tcpClients.Count -gt 0) { $blockers += "live_tcp_clients=$($tcpClients.Count)" }
    if ($null -ne $activeSessions -and $activeSessions -gt 0 -and $tcpClients.Count -gt 0) {
        $blockers += "active_sessions=$activeSessions"
    }
    if ($null -ne $activeSessions -and $activeSessions -gt 0 -and $tcpClients.Count -eq 0) {
        Info "Synapse restart guard reason=$Reason idle_session_map_entries=$activeSessions live_tcp_clients=0 verdict=not_blocking_idle_sessions"
    }
    if ($staleTcpConnections.Count -gt 0) {
        Info ("Synapse restart guard reason={0} stale_tcp_connections={1}`nstale_tcp:`n{2}" -f `
            $Reason,
            $staleTcpConnections.Count,
            (Format-SynapseTcpClientSnapshot -Snapshot $staleTcpConnections))
    }

    if ($blockers.Count -gt 0) {
        $message = ("SYNAPSE_ACTIVE_CLIENTS_PRESENT reason={0} blockers={1} process_count={2} active_sessions={3} live_tcp_clients={4} idle_session_map_entries={5} stale_tcp_connections={6}`nprocesses:`n{7}`ntcp_clients:`n{8}`nstale_tcp:`n{9}`nremediation=wait for MCP clients to disconnect, close only the exact owner-known helper process listed here, or rerun with -ForceRestart only after coordinating a maintenance window. Do not close terminal windows or broad shell processes." -f `
            $Reason,
            ($blockers -join ','),
            $processes.Count,
            ($(if ($null -eq $activeSessions) { 'unknown' } else { $activeSessions })),
            $tcpClients.Count,
            ($(if ($null -eq $activeSessions) { 'unknown' } else { $activeSessions })),
            $staleTcpConnections.Count,
            (Format-SynapseMcpProcessSnapshot -Snapshot $processes),
            (Format-SynapseTcpClientSnapshot -Snapshot $tcpClients),
            (Format-SynapseTcpClientSnapshot -Snapshot $staleTcpConnections))
        if ($ForceRestart) {
            Info "FORCE_RESTART: $message"
        } else {
            Die $message
        }
    } else {
        Info "Synapse restart guard reason=$Reason verdict=clear active_sessions=$activeSessions live_tcp_clients=0 stale_tcp_connections=$($staleTcpConnections.Count) process_count=$($processes.Count)"
    }
}

function Assert-SynapseProcessStopTarget {
    param([Parameter(Mandatory=$true)]$SnapshotProcess)

    $pidValue = [int]$SnapshotProcess.ProcessId
    $current = Get-CimInstance Win32_Process -Filter "ProcessId=$pidValue" -ErrorAction SilentlyContinue
    if (-not $current) {
        Info "Synapse process stop target already exited pid=$pidValue"
        return $null
    }

    $protectedNames = @(
        'cmd.exe',
        'powershell.exe',
        'pwsh.exe',
        'WindowsTerminal.exe',
        'OpenConsole.exe',
        'conhost.exe',
        'wsl.exe',
        'wslhost.exe',
        'Code.exe'
    )
    if ($protectedNames -contains $current.Name) {
        Die ("SYNAPSE_PROTECTED_PROCESS_STOP_REFUSED pid={0} name={1} command_line={2} remediation=terminal/IDE/WSL host processes are operator and agent workspaces; never close them from setup, tests, or FSV. Stop only exact owner-known helper PIDs." -f `
            $pidValue,
            $current.Name,
            $current.CommandLine)
    }

    $exeLeaf = if ($current.ExecutablePath) { Split-Path -Leaf $current.ExecutablePath } else { '' }
    if ($current.Name -ine 'synapse-mcp.exe' -and $exeLeaf -ine 'synapse-mcp.exe') {
        Die ("SYNAPSE_PROCESS_STOP_TARGET_MISMATCH pid={0} expected=synapse-mcp.exe actual_name={1} actual_path={2} command_line={3} remediation=PID was reused or snapshot was not a Synapse process; refusing exact-PID stop" -f `
            $pidValue,
            $current.Name,
            $current.ExecutablePath,
            $current.CommandLine)
    }
    if ($current.CommandLine -notmatch '(?i)synapse-mcp(\.exe)?') {
        Die ("SYNAPSE_PROCESS_STOP_TARGET_UNVERIFIED pid={0} name={1} command_line={2} remediation=command line does not prove a Synapse MCP target; refusing exact-PID stop" -f `
            $pidValue,
            $current.Name,
            $current.CommandLine)
    }

    return $current
}

function Stop-SynapseMcpProcesses {
    param(
        [Parameter(Mandatory=$true)][string]$Reason,
        [int]$TimeoutSeconds = 15
    )

    $before = Get-SynapseMcpProcessSnapshot
    Info "Synapse process stop requested reason=$Reason before_count=$($before.Count)"
    Info ("Synapse process stop before:`n{0}" -f (Format-SynapseMcpProcessSnapshot -Snapshot $before))
    if ($before.Count -eq 0) {
        return
    }

    foreach ($proc in $before) {
        $verified = Assert-SynapseProcessStopTarget -SnapshotProcess $proc
        if (-not $verified) { continue }
        $pidValue = [int]$verified.ProcessId
        try {
            Stop-Process -Id $pidValue -Force -ErrorAction Stop
            Info "Synapse process exact-PID stop issued pid=$pidValue reason=$Reason"
        } catch {
            Die ("SYNAPSE_PROCESS_STOP_FAILED pid={0} reason={1} error={2} remediation=setup only stops verified synapse-mcp.exe PIDs; inspect process ownership and retry after the daemon exits" -f `
                $pidValue,
                $Reason,
                $_.Exception.Message)
        }
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 250
        $after = Get-SynapseMcpProcessSnapshot
        if ($after.Count -eq 0) {
            Info "Synapse process stop verified reason=$Reason after_count=0"
            return
        }
    } while ((Get-Date) -lt $deadline)

    $remaining = Get-SynapseMcpProcessSnapshot
    Die ("SYNAPSE_PROCESS_STOP_FAILED reason={0} timeout_s={1} remaining_count={2} remaining=`n{3}" -f `
        $Reason, $TimeoutSeconds, $remaining.Count, (Format-SynapseMcpProcessSnapshot -Snapshot $remaining))
}

# ---------------------------------------------------------------------------
# Uninstall path
# ---------------------------------------------------------------------------
$maintenanceReason = if ($Remove) { 'remove' } else { 'setup' }
Acquire-SynapseSetupMaintenanceLock -Path $MaintenanceLockPath -Reason $maintenanceReason

if ($Remove) {
    Step "Removing scheduled task '$TaskName'"
    Assert-SynapseRestartAllowed -Reason 'remove' -Bind $Bind -TokenPath $TokenPath -ForceRestart:$ForceRestart
    if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask  -TaskName $TaskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        Info "Unregistered '$TaskName'."
    } else { Info "Task '$TaskName' not present." }
    Stop-SynapseMcpProcesses -Reason 'remove'
    if ($Purge) {
        foreach ($p in @($DbPath, $ProfilesDir, (Split-Path -Parent $TokenPath))) {
            if (Test-Path $p) { Remove-Item -Recurse -Force $p; Info "Deleted $p" }
        }
    }
    Info "Done (remove)."
    return
}

# ---------------------------------------------------------------------------
# 1. Preflight
# ---------------------------------------------------------------------------
Step "Preflight"
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
if (-not $SkipBuild) {
    if (-not (Test-Path $cargo)) {
        Die "cargo not found at $cargo. Install the Rust toolchain (https://rustup.rs) on Windows, then re-run. Synapse builds with the current stable toolchain."
    }
    if (-not $SourceDir) { Die "-SourceDir is required unless -SkipBuild is set." }
    if (-not (Test-Path (Join-Path $SourceDir 'Cargo.toml'))) {
        Die "-SourceDir '$SourceDir' has no Cargo.toml. Point it at a synapse source checkout on a LOCAL drive."
    }
    if ($SourceDir -match '^\\\\' -or $SourceDir -match '^[Zz]:\\home\\') {
        Die "-SourceDir '$SourceDir' looks like a UNC / WSL-mapped path. Build from a real local copy: building over \\wsl.localhost bakes transient drive paths into the binary."
    }
    Info "cargo: $((& $cargo --version))"
}

# ---------------------------------------------------------------------------
# 2. Build (local source -> persistent target) and verify the binary
# ---------------------------------------------------------------------------
if (-not $SkipBuild) {
    Step "Building synapse-mcp (release) from $SourceDir"
    New-Item -ItemType Directory -Force -Path $CargoTarget, $LogDir | Out-Null
    $env:CARGO_TARGET_DIR = $CargoTarget
    $buildLog = Join-Path $LogDir 'setup-build.log'
    Info "Build process tree is job-owned; log: $buildLog"
    $buildExit = Invoke-SynapseProcessInKillOnCloseJob `
        -FilePath $cargo `
        -ArgumentList @('build','--release','-p','synapse-mcp') `
        -WorkingDirectory $SourceDir `
        -TimeoutMinutes $BuildTimeoutMinutes `
        -LogPath $buildLog
    if ($buildExit -ne 0) {
        $tail = if (Test-Path $buildLog) { (Get-Content -Path $buildLog -Tail 80 -ErrorAction SilentlyContinue) -join "`n" } else { '' }
        Die "cargo build failed (exit $buildExit). Build log: $buildLog. Tail:`n$tail"
    }
    $built = Join-Path $CargoTarget 'release\synapse-mcp.exe'
    if (-not (Test-Path $built)) { Die "Build reported success but $built is missing." }
    Info "Built: $built ($([math]::Round((Get-Item $built).Length/1MB,1)) MB)"
}

# ---------------------------------------------------------------------------
# 3. Stop the running daemon/bridges so the .exe is not locked, then install
# ---------------------------------------------------------------------------
Step "Installing daemon binary -> $ExePath"
Assert-SynapseRestartAllowed -Reason 'install_binary' -Bind $Bind -TokenPath $TokenPath -ForceRestart:$ForceRestart
if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
}
Stop-SynapseMcpProcesses -Reason 'install_binary'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ExePath) | Out-Null
if (-not $SkipBuild) {
    if (Test-Path $ExePath) { Copy-Item $ExePath "$ExePath.bak" -Force; Info "Backed up old binary -> $ExePath.bak" }
    Copy-Item (Join-Path $CargoTarget 'release\synapse-mcp.exe') $ExePath -Force
}
if (-not (Test-Path $ExePath)) { Die "No daemon binary at $ExePath (build skipped and none installed)." }
$ver = (& $ExePath --version) 2>&1
Info "Installed binary reports: $ver"

# ---------------------------------------------------------------------------
# 4. Deploy bundled profiles next to the exe (executable-relative lookup) +
#    keep an explicit --profile-dir for belt-and-suspenders.
# ---------------------------------------------------------------------------
Step "Deploying bundled profiles -> $ProfilesDir"
$srcProfiles = if ($SourceDir) { Join-Path $SourceDir 'crates\synapse-profiles\profiles' } else { $null }
if ($srcProfiles -and (Test-Path $srcProfiles)) {
    New-Item -ItemType Directory -Force -Path $ProfilesDir | Out-Null
    Copy-Item "$srcProfiles\*" $ProfilesDir -Recurse -Force
    $n = (Get-ChildItem $ProfilesDir -Filter *.toml -File).Count
    if ($n -lt 1) { Die "Copied profiles but found 0 .toml files in $ProfilesDir." }
    Info "Deployed $n profiles."
} elseif (-not (Test-Path $ProfilesDir)) {
    Die "No bundled profiles found (source '$srcProfiles' missing and $ProfilesDir absent). Profile-dependent tools (reflexes, action policy) need these."
} else { Info "Reusing existing profiles at $ProfilesDir." }

# ---------------------------------------------------------------------------
# 5. Token, DB and log dirs
# ---------------------------------------------------------------------------
Step "Bearer token + data dirs"
$tokDir = Split-Path -Parent $TokenPath
New-Item -ItemType Directory -Force -Path $tokDir, $DbPath, $LogDir | Out-Null
if (-not (Test-Path $TokenPath)) {
    $bytes = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    ($bytes | ForEach-Object { $_.ToString('x2') }) -join '' | Set-Content -Path $TokenPath -NoNewline -Encoding ascii
    Info "Generated token -> $TokenPath"
} else { Info "Reusing token -> $TokenPath" }
$tokenRaw = Get-Content -Raw $TokenPath
$token = if ($null -eq $tokenRaw) { '' } else { $tokenRaw.Trim() }
if ($token.Length -lt 16) { Die "Token at $TokenPath is too short ($($token.Length) chars); delete it and re-run to regenerate." }
[Environment]::SetEnvironmentVariable('SYNAPSE_BEARER_TOKEN', $token, 'User')
$env:SYNAPSE_BEARER_TOKEN = $token
Info "Set Windows User SYNAPSE_BEARER_TOKEN from $TokenPath for native HTTP MCP clients that require env-based bearer auth."
try {
    $signature = '[DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'
    $type = Add-Type -MemberDefinition $signature -Name Win32SendMessageTimeout -Namespace SynapseEnv -PassThru -ErrorAction Stop
    $broadcastResult = [UIntPtr]::Zero
    $rawReturn = $type::SendMessageTimeout([IntPtr]0xffff, 0x001A, [UIntPtr]::Zero, 'Environment', 0x0002, 5000, [ref]$broadcastResult)
    if ($rawReturn -eq [IntPtr]::Zero) {
        Info "WARN: environment broadcast returned 0; future GUI clients may need restart before seeing SYNAPSE_BEARER_TOKEN."
    }
} catch {
    Info "WARN: environment broadcast failed: $($_.Exception.Message). Future GUI clients may need restart before seeing SYNAPSE_BEARER_TOKEN."
}

# ---------------------------------------------------------------------------
# 6. Register + start the auto-start HTTP daemon (interactive desktop session)
# ---------------------------------------------------------------------------
Step "Registering auto-start daemon task '$TaskName'"
$legacyLauncher = Join-Path $LogDir 'synapse-daemon-launch.cmd'
$hiddenLauncher = Join-Path $LogDir 'synapse-daemon-launch-hidden.vbs'
$launcherLog = Join-Path $LogDir 'daemon-launcher.log'
if (Test-Path $legacyLauncher) {
    Remove-Item -LiteralPath $legacyLauncher -Force
}
$wscriptExe = Join-Path $env:SystemRoot 'System32\wscript.exe'
if (-not (Test-Path $wscriptExe)) {
    Die "SYNAPSE_HIDDEN_LAUNCHER_MISSING path=$wscriptExe remediation=repair Windows Script Host or run the daemon manually with a hidden process supervisor"
}
New-HiddenDaemonLauncher -OutputPath $hiddenLauncher -ExePath $ExePath -Bind $Bind -DbPath $DbPath -ProfilesDir $ProfilesDir -LogDir $LogDir -TokenPath $TokenPath

$action  = New-ScheduledTaskAction -Execute $wscriptExe -Argument "//B //Nologo `"$hiddenLauncher`"" -WorkingDirectory $LogDir
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$princ   = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$set     = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
            -StartWhenAvailable -MultipleInstances IgnoreNew -RestartCount 3 `
            -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero)
$set.Hidden = $true
if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
}
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $princ `
    -Settings $set -Description "Synapse MCP HTTP daemon (loopback) - the single body controlling Windows + WSL programs." | Out-Null
Start-ScheduledTask -TaskName $TaskName
Info "Task registered and started."

# ---------------------------------------------------------------------------
# 7. Health verify (source of truth: the live daemon)
# ---------------------------------------------------------------------------
Step "Verifying daemon health (http://$Bind/health)"
$ok = $false
$healthPid = $null
for ($i=0; $i -lt 15; $i++) {
    Start-Sleep -Seconds 2
    try {
        $h = Invoke-RestMethod -Uri "http://$Bind/health" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 4
        if ($h.ok) {
            Info ("Daemon OK: pid={0} version={1} db={2}" -f $h.pid, $h.version, $h.subsystems.storage.db_path)
            $healthPid = [int]$h.pid
            $ok = $true; break
        }
    } catch { }
}
if (-not $ok) { Die "Daemon did not become healthy on http://$Bind/health. Check $launcherLog and synapse.log.* under $LogDir for launch / STORAGE_* / bind errors." }
$daemonLineage = Get-ProcessLineage -StartPid $healthPid
$cmdAncestor = $daemonLineage | Where-Object { $_.Name -ieq 'cmd.exe' } | Select-Object -First 1
if ($cmdAncestor) {
    $lineageText = ($daemonLineage | ForEach-Object { "{0}:{1}" -f $_.ProcessId, $_.Name }) -join ' <- '
    Die "SYNAPSE_DAEMON_CMD_ANCESTOR_FORBIDDEN pid=$healthPid cmd_pid=$($cmdAncestor.ProcessId) lineage=$lineageText remediation=rerun setup after removing legacy daemon launchers; daemon must not be launched through cmd.exe."
}

# ---------------------------------------------------------------------------
# 8. Wire the Windows-side MCP clients
# ---------------------------------------------------------------------------
if (-not $SkipClientWiring) {
    Step "Wiring Windows-side MCP clients"

    # Claude Code (Windows) speaks Streamable HTTP natively -> point at the daemon.
    $claude = Get-Command claude -ErrorAction SilentlyContinue
    if ($claude) {
        try {
            & $claude.Source mcp remove synapse -s user 2>$null | Out-Null
            & $claude.Source mcp add --scope user --transport http synapse "http://$Bind/mcp" --header "Authorization: Bearer $token"
            Info "Claude Code (Windows) wired via HTTP transport."
        } catch { Info "WARN: 'claude mcp add' failed: $($_.Exception.Message). Wire it manually (transport http -> http://$Bind/mcp)." }
    } else { Info "claude CLI not found on Windows PATH; skipping Claude Code wiring." }

    # Codex speaks Streamable HTTP; Claude Desktop remains stdio-only -> connect bridge.
    $bridgeArgs = @('--mode','connect','--bind',$Bind)

    $codex = Get-Command codex -ErrorAction SilentlyContinue
    $codexCfg = "$env:USERPROFILE\.codex\config.toml"
    if ($codex) {
        & $codex.Source mcp remove synapse 2>$null | Out-Null
        & $codex.Source mcp add synapse --url "http://$Bind/mcp" --bearer-token-env-var SYNAPSE_BEARER_TOKEN
        if ($LASTEXITCODE -ne 0) { Die "codex mcp add failed (exit $LASTEXITCODE). Codex must be wired to HTTP, not the connect bridge." }
        Install-CodexSynapseTokenLoader -CodexCommandPath $codex.Source -TokenPath $TokenPath
        Info "Codex (Windows) wired via Streamable HTTP transport."
    } elseif (Test-Path $codexCfg) {
        $c = Get-Content -Raw $codexCfg
        $bindUrlRegex = [regex]::Escape("http://$Bind/mcp")
        if ($c -match '(?m)^\[mcp_servers\.synapse\]' -and ($c -notmatch "url\s*=\s*`"$bindUrlRegex`"" -or $c -notmatch 'bearer_token_env_var\s*=\s*"SYNAPSE_BEARER_TOKEN"')) {
            Die "Codex config exists at $codexCfg but codex CLI is not on PATH and the synapse entry is not the required HTTP transport. Install/repair Codex CLI, then re-run."
        }
        Info "Codex CLI not found; existing Codex config is already HTTP or has no synapse entry."
    } else { Info "codex CLI/config not found; skipping Codex wiring." }

    $desktopCfg = "$env:APPDATA\Claude\claude_desktop_config.json"
    if (Test-Path $desktopCfg) {
        try {
            $j = Get-Content -Raw $desktopCfg | ConvertFrom-Json
            if (-not $j.mcpServers) { $j | Add-Member -NotePropertyName mcpServers -NotePropertyValue (@{}) -Force }
            $j.mcpServers.synapse = @{ command = $ExePath; args = $bridgeArgs; env = @{ SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY = '1' } }
            ($j | ConvertTo-Json -Depth 12) | Set-Content $desktopCfg -Encoding utf8
            Info "Claude Desktop wired -> connect bridge."
        } catch { Info "WARN: could not update $desktopCfg : $($_.Exception.Message)" }
    } else { Info "No Claude Desktop config at $desktopCfg; skipping." }
}

$lineage = Get-ProcessLineage
$codexAncestor = $lineage | Where-Object {
    $_.Name -ieq 'codex.exe' -or $_.CommandLine -match '@openai[\\/]+codex|codex\.js|codex-win32'
} | Select-Object -First 1
if ($codexAncestor -and $processTokenAtStart -ne $token) {
    Die ("SYNAPSE_CODEX_CURRENT_PROCESS_ENV_STALE codex_pid={0} token_at_process_start={1} token_file={2} remediation=restart Codex through the patched codex launcher; Windows cannot update an already-running Codex process environment, so this current session cannot authenticate mcp__synapse yet." -f $codexAncestor.ProcessId, ($(if ([string]::IsNullOrWhiteSpace($processTokenAtStart)) { 'missing' } else { 'mismatch' })), $TokenPath)
}

Step "Done"
Info "Synapse daemon is live on http://$Bind (MCP: http://$Bind/mcp)."
Info "Token: $TokenPath   DB: $DbPath   Profiles: $ProfilesDir"
Info "WSL clients: run scripts/synapse-install.sh from WSL to wire Claude Code + Codex there."
