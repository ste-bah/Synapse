const VK_A: u32 = 0x41;
const WM_KEYDOWN_RAW: u32 = 0x0100;
const WM_KEYUP_RAW: u32 = 0x0101;
const SIGINT_RELEASE_BUDGET_MS: u128 = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
struct KeyEventReadback {
    elapsed_ms: u128,
    vk_code: u32,
    message: u32,
    flags: u32,
}

impl KeyEventReadback {
    const fn new(elapsed_ms: u128, vk_code: u32, message: u32, flags: u32) -> Self {
        Self {
            elapsed_ms,
            vk_code,
            message,
            flags,
        }
    }

    const fn is_a_keyup(&self) -> bool {
        self.vk_code == VK_A && self.message == WM_KEYUP_RAW
    }
}

#[test]
fn keyup_latency_detects_first_a_release_after_sigint() {
    let before = vec![
        KeyEventReadback::new(5, VK_A, WM_KEYDOWN_RAW, 0),
        KeyEventReadback::new(7, VK_A, WM_KEYUP_RAW, 0),
        KeyEventReadback::new(20, VK_A, WM_KEYUP_RAW, 0),
    ];
    let sigint_at_ms = 12;
    println!(
        "source_of_truth=keyboard_hook_timeline edge=latency_happy before={} sigint_at_ms={sigint_at_ms}",
        format_timeline(&before)
    );
    let after = keyup_latency_after_sigint(&before, sigint_at_ms);
    println!(
        "source_of_truth=keyboard_hook_timeline edge=latency_happy after_latency_ms={after:?}"
    );
    assert_eq!(after, Some(8));
}

#[test]
fn keyup_latency_rejects_only_pre_sigint_release() {
    let before = vec![
        KeyEventReadback::new(1, VK_A, WM_KEYDOWN_RAW, 0),
        KeyEventReadback::new(4, VK_A, WM_KEYUP_RAW, 0),
    ];
    let sigint_at_ms = 10;
    println!(
        "source_of_truth=keyboard_hook_timeline edge=pre_sigint_only before={} sigint_at_ms={sigint_at_ms}",
        format_timeline(&before)
    );
    let after = keyup_latency_after_sigint(&before, sigint_at_ms);
    println!(
        "source_of_truth=keyboard_hook_timeline edge=pre_sigint_only after_latency_ms={after:?}"
    );
    assert_eq!(after, None);
}

#[test]
fn keyup_latency_rejects_wrong_key_after_sigint() {
    let before = vec![
        KeyEventReadback::new(1, VK_A, WM_KEYDOWN_RAW, 0),
        KeyEventReadback::new(12, 0x42, WM_KEYUP_RAW, 0),
    ];
    let sigint_at_ms = 10;
    println!(
        "source_of_truth=keyboard_hook_timeline edge=wrong_key before={} sigint_at_ms={sigint_at_ms}",
        format_timeline(&before)
    );
    let after = keyup_latency_after_sigint(&before, sigint_at_ms);
    println!("source_of_truth=keyboard_hook_timeline edge=wrong_key after_latency_ms={after:?}");
    assert_eq!(after, None);
}

#[test]
fn release_budget_boundary_is_inclusive_at_ten_ms() {
    let timeline = vec![
        KeyEventReadback::new(100, VK_A, WM_KEYDOWN_RAW, 0),
        KeyEventReadback::new(110, VK_A, WM_KEYUP_RAW, 0),
    ];
    println!(
        "source_of_truth=keyboard_hook_timeline edge=budget_boundary before={} sigint_at_ms=100",
        format_timeline(&timeline)
    );
    let latency = keyup_latency_after_sigint(&timeline, 100)
        .unwrap_or_else(|| panic!("expected keyup at the budget boundary"));
    println!(
        "source_of_truth=keyboard_hook_timeline edge=budget_boundary after_latency_ms={latency}"
    );
    assert!(latency <= SIGINT_RELEASE_BUDGET_MS);
}

fn keyup_latency_after_sigint(timeline: &[KeyEventReadback], sigint_at_ms: u128) -> Option<u128> {
    timeline
        .iter()
        .filter(|event| event.is_a_keyup() && event.elapsed_ms >= sigint_at_ms)
        .map(|event| event.elapsed_ms - sigint_at_ms)
        .min()
}

fn format_timeline(timeline: &[KeyEventReadback]) -> String {
    timeline
        .iter()
        .map(|event| {
            format!(
                "t={}ms vk=0x{:02x} message={} flags=0x{:x}",
                event.elapsed_ms,
                event.vk_code,
                message_label(event.message),
                event.flags
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

const fn message_label(message: u32) -> &'static str {
    match message {
        WM_KEYDOWN_RAW => "WM_KEYDOWN",
        WM_KEYUP_RAW => "WM_KEYUP",
        _ => "OTHER",
    }
}

#[cfg(windows)]
mod windows_fsv {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
        process::{Command, Output, Stdio},
        sync::{Mutex, MutexGuard, OnceLock},
        thread,
        time::{Duration, Instant},
    };

    use std::os::windows::process::CommandExt;

    use anyhow::{Context, anyhow, bail};
    use tempfile::TempDir;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    const POWERSHELL_TIMEOUT: Duration = Duration::from_secs(25);

    use super::SIGINT_RELEASE_BUDGET_MS;

    fn desktop_fsv_lock() -> anyhow::Result<MutexGuard<'static, ()>> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_err| anyhow!("desktop FSV lock poisoned"))
    }

    #[test]
    #[ignore = "requires native Windows desktop, WH_KEYBOARD_LL, and console Ctrl-C delivery"]
    fn wh_keyboard_ll_observes_keyup_within_10ms_after_sigint_fsv() -> anyhow::Result<()> {
        let _guard = desktop_fsv_lock()?;
        let work_dir = TempDir::new()?;
        let script_path = work_dir.path().join("sigint_keyboard_hook_fsv.ps1");
        std::fs::write(&script_path, POWERSHELL_HOOK_SCRIPT)?;
        let output =
            run_powershell_fsv(&script_path, work_dir.path(), &mcp_binary_path()?, "sigint")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        println!("{}", stdout.trim_end());
        if !stderr.trim().is_empty() {
            println!("source_of_truth=powershell_stderr edge=sigint after={stderr:?}");
        }

        if !output.status.success() {
            bail!(
                "PowerShell WH_KEYBOARD_LL FSV failed with status {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                output.status.code()
            );
        }
        assert!(stdout.contains("source_of_truth=keyboard_hook_timeline edge=sigint before="));
        assert!(
            stdout.contains("source_of_truth=keyboard_hook_timeline edge=sigint before_sigint=")
        );
        assert!(stdout.contains("source_of_truth=keyboard_hook_timeline edge=sigint after="));
        assert!(stdout.contains("source_of_truth=daemon_log edge=sigint after_exit=0"));
        assert!(stdout.contains("\"reason\":\"sigint\""));
        assert!(stdout.contains("\"released_keys\":1"));
        Ok(())
    }

    #[test]
    #[ignore = "requires native Windows desktop, WH_KEYBOARD_LL, and stdio child process control"]
    fn wh_keyboard_ll_observes_keyup_within_10ms_after_connection_closed_fsv() -> anyhow::Result<()>
    {
        let _guard = desktop_fsv_lock()?;
        let work_dir = TempDir::new()?;
        let script_path = work_dir
            .path()
            .join("connection_closed_keyboard_hook_fsv.ps1");
        std::fs::write(&script_path, POWERSHELL_HOOK_SCRIPT)?;
        let output = run_powershell_fsv(
            &script_path,
            work_dir.path(),
            &mcp_binary_path()?,
            "connection_closed",
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        println!("{}", stdout.trim_end());
        if !stderr.trim().is_empty() {
            println!("source_of_truth=powershell_stderr edge=connection_closed after={stderr:?}");
        }

        if !output.status.success() {
            bail!(
                "PowerShell connection_closed WH_KEYBOARD_LL FSV failed with status {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                output.status.code()
            );
        }
        assert!(
            stdout
                .contains("source_of_truth=keyboard_hook_timeline edge=connection_closed before=")
        );
        assert!(
            stdout.contains(
                "source_of_truth=keyboard_hook_timeline edge=connection_closed before_connection_closed="
            )
        );
        assert!(
            stdout.contains("source_of_truth=keyboard_hook_timeline edge=connection_closed after=")
        );
        assert!(stdout.contains("source_of_truth=daemon_log edge=connection_closed after_exit=0"));
        assert!(stdout.contains("\"reason\":\"connection_closed\""));
        assert!(stdout.contains("\"released_keys\":1"));
        Ok(())
    }

    #[test]
    #[ignore = "requires native Windows desktop, WH_KEYBOARD_LL, and stdio child process control"]
    fn wh_keyboard_ll_observes_keyup_within_10ms_after_panic_mid_press_fsv() -> anyhow::Result<()> {
        let _guard = desktop_fsv_lock()?;
        let work_dir = TempDir::new()?;
        let script_path = work_dir.path().join("panic_keyboard_hook_fsv.ps1");
        std::fs::write(&script_path, POWERSHELL_HOOK_SCRIPT)?;
        let output =
            run_powershell_fsv(&script_path, work_dir.path(), &mcp_binary_path()?, "panic")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        println!("{}", stdout.trim_end());
        if !stderr.trim().is_empty() {
            println!("source_of_truth=powershell_stderr edge=panic after={stderr:?}");
        }

        if !output.status.success() {
            bail!(
                "PowerShell panic WH_KEYBOARD_LL FSV failed with status {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                output.status.code()
            );
        }
        assert!(stdout.contains("source_of_truth=keyboard_hook_timeline edge=panic before="));
        assert!(stdout.contains("source_of_truth=keyboard_hook_timeline edge=panic before_panic="));
        assert!(stdout.contains("source_of_truth=keyboard_hook_timeline edge=panic after="));
        assert!(stdout.contains("source_of_truth=daemon_log edge=panic after_exit="));
        assert!(stdout.contains("after_release_line="));
        assert!(stdout.contains("source_of_truth=daemon_log edge=panic after_panic_line="));
        assert!(stdout.contains("\"reason\":\"tool_invocation\""));
        assert!(stdout.contains("\"reason\":\"panic\""));
        assert!(stdout.contains("\"released_keys\":1"));
        Ok(())
    }

    fn run_powershell_fsv(
        script_path: &Path,
        work_dir: &Path,
        mcp_bin: &Path,
        mode: &str,
    ) -> anyhow::Result<Output> {
        let mut child = Command::new(powershell_exe())
            .args([
                OsString::from("-NoProfile"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
                script_path.as_os_str().to_owned(),
                OsString::from("-McpBin"),
                mcp_bin.as_os_str().to_owned(),
                OsString::from("-WorkDir"),
                work_dir.as_os_str().to_owned(),
                OsString::from("-BudgetMs"),
                OsString::from(SIGINT_RELEASE_BUDGET_MS.to_string()),
                OsString::from("-Mode"),
                OsString::from(mode),
            ])
            .creation_flags(CREATE_NEW_CONSOLE)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn PowerShell WH_KEYBOARD_LL helper")?;
        let deadline = Instant::now() + POWERSHELL_TIMEOUT;
        loop {
            if child
                .try_wait()
                .context("poll PowerShell helper")?
                .is_some()
            {
                return child
                    .wait_with_output()
                    .context("collect PowerShell helper output");
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .context("collect timed-out PowerShell helper output")?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!(
                    "timed out after {POWERSHELL_TIMEOUT:?} waiting for PowerShell WH_KEYBOARD_LL helper\nstdout:\n{stdout}\nstderr:\n{stderr}"
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn powershell_exe() -> OsString {
        std::env::var_os("SYNAPSE_POWERSHELL").unwrap_or_else(|| OsString::from("powershell.exe"))
    }

    fn mcp_binary_path() -> anyhow::Result<PathBuf> {
        if let Some(path) = std::env::var_os("SYNAPSE_MCP_BIN") {
            return Ok(PathBuf::from(path));
        }
        std::env::var_os("CARGO_BIN_EXE_synapse-mcp")
            .map(PathBuf::from)
            .context("CARGO_BIN_EXE_synapse-mcp is unset; set SYNAPSE_MCP_BIN or run this as a synapse-mcp integration test")
    }

    const POWERSHELL_HOOK_SCRIPT: &str = r#"
param(
  [Parameter(Mandatory=$true)][string]$McpBin,
  [Parameter(Mandatory=$true)][string]$WorkDir,
  [Parameter(Mandatory=$true)][int]$BudgetMs,
  [ValidateSet('sigint', 'connection_closed', 'panic')]
  [string]$Mode = 'sigint'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public sealed class KeyEventRecord
{
    public long ElapsedMs { get; set; }
    public int VkCode { get; set; }
    public int Message { get; set; }
    public int Flags { get; set; }

    public bool IsAKeyDown()
    {
        return VkCode == 0x41 && Message == 0x0100;
    }

    public bool IsAKeyUp()
    {
        return VkCode == 0x41 && Message == 0x0101;
    }
}

public static class KeyboardHookRecorder
{
    private const int WH_KEYBOARD_LL = 13;
    private const int WM_KEYDOWN = 0x0100;
    private const int WM_KEYUP = 0x0101;
    private const int WM_SYSKEYDOWN = 0x0104;
    private const int WM_SYSKEYUP = 0x0105;
    private const int WM_QUIT = 0x0012;
    private static readonly object Gate = new object();
    private static readonly List<KeyEventRecord> Events = new List<KeyEventRecord>();
    private static readonly LowLevelKeyboardProc Callback = HookCallback;
    private static Thread HookThread;
    private static IntPtr HookId = IntPtr.Zero;
    private static uint HookThreadId;
    private static long StartTimestamp;

    public static void Start()
    {
        Stop();
        lock (Gate)
        {
            Events.Clear();
            StartTimestamp = Stopwatch.GetTimestamp();
        }

        ManualResetEventSlim ready = new ManualResetEventSlim(false);
        Exception failure = null;
        HookThread = new Thread(() =>
        {
            try
            {
                HookThreadId = GetCurrentThreadId();
                IntPtr module = GetModuleHandle(null);
                HookId = SetWindowsHookEx(WH_KEYBOARD_LL, Callback, module, 0);
                if (HookId == IntPtr.Zero)
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "SetWindowsHookEx WH_KEYBOARD_LL failed");
                }
                ready.Set();
                MSG message;
                int result;
                while ((result = GetMessage(out message, IntPtr.Zero, 0, 0)) > 0) { }
                if (result < 0)
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "GetMessage failed");
                }
            }
            catch (Exception ex)
            {
                failure = ex;
                ready.Set();
            }
            finally
            {
                if (HookId != IntPtr.Zero)
                {
                    UnhookWindowsHookEx(HookId);
                    HookId = IntPtr.Zero;
                }
            }
        });
        HookThread.SetApartmentState(ApartmentState.STA);
        HookThread.IsBackground = true;
        HookThread.Start();

        if (!ready.Wait(TimeSpan.FromSeconds(5)))
        {
            throw new TimeoutException("timed out waiting for WH_KEYBOARD_LL hook");
        }
        if (failure != null)
        {
            throw new InvalidOperationException("WH_KEYBOARD_LL hook failed", failure);
        }
    }

    public static void Stop()
    {
        Thread thread = HookThread;
        if (thread == null)
        {
            return;
        }
        PostThreadMessage(HookThreadId, WM_QUIT, IntPtr.Zero, IntPtr.Zero);
        thread.Join(TimeSpan.FromSeconds(5));
        HookThread = null;
    }

    public static long ElapsedMs()
    {
        long delta = Stopwatch.GetTimestamp() - StartTimestamp;
        return (long)Math.Floor(delta * 1000.0 / Stopwatch.Frequency);
    }

    public static KeyEventRecord[] Snapshot()
    {
        lock (Gate)
        {
            return Events.ToArray();
        }
    }

    public static string FormatTimeline()
    {
        KeyEventRecord[] snapshot = Snapshot();
        if (snapshot.Length == 0)
        {
            return "<empty>";
        }
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < snapshot.Length; i++)
        {
            if (i > 0)
            {
                builder.Append("; ");
            }
            KeyEventRecord ev = snapshot[i];
            builder.Append("t=").Append(ev.ElapsedMs).Append("ms ");
            builder.Append("vk=0x").Append(ev.VkCode.ToString("x2")).Append(" ");
            builder.Append("message=").Append(MessageLabel(ev.Message)).Append(" ");
            builder.Append("flags=0x").Append(ev.Flags.ToString("x"));
        }
        return builder.ToString();
    }

    private static IntPtr HookCallback(int code, IntPtr wparam, IntPtr lparam)
    {
        if (code >= 0)
        {
            int message = wparam.ToInt32();
            if (message == WM_KEYDOWN || message == WM_KEYUP || message == WM_SYSKEYDOWN || message == WM_SYSKEYUP)
            {
                KBDLLHOOKSTRUCT data = (KBDLLHOOKSTRUCT)Marshal.PtrToStructure(lparam, typeof(KBDLLHOOKSTRUCT));
                if (data.vkCode == 0x41)
                {
                    lock (Gate)
                    {
                        Events.Add(new KeyEventRecord {
                            ElapsedMs = ElapsedMs(),
                            VkCode = data.vkCode,
                            Message = message,
                            Flags = data.flags
                        });
                    }
                }
            }
        }
        return CallNextHookEx(HookId, code, wparam, lparam);
    }

    private static string MessageLabel(int message)
    {
        if (message == WM_KEYDOWN) return "WM_KEYDOWN";
        if (message == WM_KEYUP) return "WM_KEYUP";
        if (message == WM_SYSKEYDOWN) return "WM_SYSKEYDOWN";
        if (message == WM_SYSKEYUP) return "WM_SYSKEYUP";
        return "OTHER";
    }

    private delegate IntPtr LowLevelKeyboardProc(int nCode, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    private struct KBDLLHOOKSTRUCT
    {
        public int vkCode;
        public int scanCode;
        public int flags;
        public int time;
        public IntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MSG
    {
        public IntPtr hwnd;
        public int message;
        public IntPtr wParam;
        public IntPtr lParam;
        public int time;
        public int pt_x;
        public int pt_y;
    }

    [DllImport("user32.dll", SetLastError=true)]
    private static extern IntPtr SetWindowsHookEx(int idHook, LowLevelKeyboardProc lpfn, IntPtr hMod, uint dwThreadId);

    [DllImport("user32.dll", SetLastError=true)]
    private static extern bool UnhookWindowsHookEx(IntPtr hhk);

    [DllImport("user32.dll")]
    private static extern IntPtr CallNextHookEx(IntPtr hhk, int nCode, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError=true)]
    private static extern int GetMessage(out MSG lpMsg, IntPtr hWnd, uint wMsgFilterMin, uint wMsgFilterMax);

    [DllImport("user32.dll", SetLastError=true)]
    private static extern bool PostThreadMessage(uint idThread, int msg, IntPtr wParam, IntPtr lParam);

    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();

    [DllImport("kernel32.dll", CharSet=CharSet.Auto, SetLastError=true)]
    private static extern IntPtr GetModuleHandle(string lpModuleName);
}

public static class ConsoleCtrl
{
    private delegate bool HandlerRoutine(uint dwCtrlType);
    private static readonly HandlerRoutine IgnoreHandler = IgnoreCtrlSignal;

    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool GenerateConsoleCtrlEvent(uint dwCtrlEvent, uint dwProcessGroupId);

    [DllImport("kernel32.dll", SetLastError=true)]
    private static extern bool SetConsoleCtrlHandler(HandlerRoutine handlerRoutine, bool add);

    public static bool InstallIgnoreHandler()
    {
        return SetConsoleCtrlHandler(IgnoreHandler, true);
    }

    public static bool RemoveIgnoreHandler()
    {
        return SetConsoleCtrlHandler(IgnoreHandler, false);
    }

    private static bool IgnoreCtrlSignal(uint dwCtrlType)
    {
        return dwCtrlType == 0 || dwCtrlType == 1;
    }
}
'@

function Write-RawJsonLine([System.Diagnostics.Process]$Process, [string]$Line) {
  $Process.StandardInput.WriteLine($Line)
  $Process.StandardInput.Flush()
}

function Wait-ForHookEvent([scriptblock]$Predicate, [int]$TimeoutMs) {
  $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
  while ([DateTime]::UtcNow -lt $deadline) {
    $events = [KeyboardHookRecorder]::Snapshot()
    foreach ($event in $events) {
      if (& $Predicate $event) {
        return
      }
    }
    Start-Sleep -Milliseconds 1
  }
  throw "timed out waiting for keyboard hook event; timeline=$([KeyboardHookRecorder]::FormatTimeline())"
}

function Get-KeyupLatencyAfterSigint([long]$SigintAtMs) {
  $latencies = @()
  foreach ($event in [KeyboardHookRecorder]::Snapshot()) {
    if ($event.IsAKeyUp() -and $event.ElapsedMs -ge $SigintAtMs) {
      $latencies += ($event.ElapsedMs - $SigintAtMs)
    }
  }
  if ($latencies.Count -eq 0) {
    return $null
  }
  return ($latencies | Measure-Object -Minimum).Minimum
}

function Get-FirstAKeyDownElapsed() {
  $latencies = @()
  foreach ($event in [KeyboardHookRecorder]::Snapshot()) {
    if ($event.IsAKeyDown()) {
      $latencies += $event.ElapsedMs
    }
  }
  if ($latencies.Count -eq 0) {
    return $null
  }
  return ($latencies | Measure-Object -Minimum).Minimum
}

function Read-SafetyLine([string]$LogDir, [string]$Reason) {
  $lines = @()
  foreach ($file in Get-ChildItem -LiteralPath $LogDir -File) {
    foreach ($line in [System.IO.File]::ReadLines($file.FullName)) {
      if ([string]::IsNullOrWhiteSpace($line)) {
        continue
      }
      try {
        $json = $line | ConvertFrom-Json
        if ($json.fields.code -eq 'SAFETY_RELEASE_ALL_FIRED' -and $json.fields.reason -eq $Reason) {
          $lines += $line
        }
      } catch {
      }
    }
  }
  if ($lines.Count -eq 0) {
    throw "expected SAFETY_RELEASE_ALL_FIRED reason=$Reason in daemon logs"
  }
  return $lines[$lines.Count - 1]
}

function Read-ReleasedKeyLine([string]$LogDir) {
  $lines = @()
  foreach ($file in Get-ChildItem -LiteralPath $LogDir -File) {
    foreach ($line in [System.IO.File]::ReadLines($file.FullName)) {
      if ([string]::IsNullOrWhiteSpace($line)) {
        continue
      }
      try {
        $json = $line | ConvertFrom-Json
        if (
          $json.fields.code -eq 'SAFETY_RELEASE_ALL_FIRED' -and
          $json.fields.reason -eq 'tool_invocation' -and
          $json.fields.released_keys -eq 1
        ) {
          $lines += $line
        }
      } catch {
      }
    }
  }
  if ($lines.Count -eq 0) {
    throw 'expected SAFETY_RELEASE_ALL_FIRED reason=tool_invocation released_keys=1 in daemon logs'
  }
  return $lines[$lines.Count - 1]
}

$proc = $null
$ignoreCtrlC = $false
try {
  [KeyboardHookRecorder]::Start()
  $logDir = Join-Path $WorkDir 'logs'
  New-Item -ItemType Directory -Force -Path $logDir | Out-Null

  $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
  $startInfo.FileName = $McpBin
  $startInfo.Arguments = '--mode stdio'
  $startInfo.UseShellExecute = $false
  $startInfo.RedirectStandardInput = $true
  $startInfo.RedirectStandardOutput = $true
  $startInfo.RedirectStandardError = $true
  $startInfo.CreateNoWindow = $false
  $startInfo.EnvironmentVariables['SYNAPSE_LOG_LEVEL'] = 'debug'
  $startInfo.EnvironmentVariables['SYNAPSE_LOG_DIR'] = $logDir
  if ($Mode -eq 'panic') {
    $startInfo.EnvironmentVariables['SYNAPSE_MCP_FORCE_PANIC_DURING_ACT'] = 'act_press_after_keydown'
  }
  $proc = [System.Diagnostics.Process]::Start($startInfo)

  Write-RawJsonLine $proc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synapse-sigint-keyboard-hook-fsv","version":"0.1.0"}}}'
  $initLine = $proc.StandardOutput.ReadLine()
  $init = $initLine | ConvertFrom-Json
  if ($init.result.serverInfo.name -ne 'synapse-mcp') {
    throw "unexpected initialize response: $initLine"
  }
  Write-RawJsonLine $proc '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'

  Write-Output "source_of_truth=keyboard_hook_timeline edge=$Mode before=$([KeyboardHookRecorder]::FormatTimeline())"
  Write-RawJsonLine $proc '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"act_press","arguments":{"keys":["a"],"hold_ms":5000,"backend":"software"}}}'
  Wait-ForHookEvent { param($event) $event.IsAKeyDown() } 2000
  Write-Output "source_of_truth=keyboard_hook_timeline edge=$Mode before_$Mode=$([KeyboardHookRecorder]::FormatTimeline())"

  if ($Mode -eq 'panic') {
    $triggerAtMs = Get-FirstAKeyDownElapsed
    if ($null -eq $triggerAtMs) {
      throw 'expected A KeyDown before forced panic'
    }
  } else {
    $triggerAtMs = [KeyboardHookRecorder]::ElapsedMs()
  }
  if ($Mode -eq 'sigint') {
    if (-not [ConsoleCtrl]::InstallIgnoreHandler()) {
      throw 'SetConsoleCtrlHandler custom ignore handler failed'
    }
    $ignoreCtrlC = $true
    if (-not [ConsoleCtrl]::GenerateConsoleCtrlEvent(1, 0)) {
      throw 'GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0) failed'
    }
  } elseif ($Mode -eq 'connection_closed') {
    $proc.StandardInput.Close()
  } elseif ($Mode -eq 'panic') {
    # The debug panic is injected inside act_press immediately after keydown.
  } else {
    throw "unsupported mode $Mode"
  }
  Wait-ForHookEvent { param($event) $event.IsAKeyUp() -and $event.ElapsedMs -ge $triggerAtMs } 2000
  $latency = Get-KeyupLatencyAfterSigint $triggerAtMs
  if ($null -eq $latency) {
    throw "expected A KeyUp after $Mode"
  }
  Write-Output "source_of_truth=keyboard_hook_timeline edge=$Mode after=$([KeyboardHookRecorder]::FormatTimeline()) trigger_at_ms=$triggerAtMs keyup_latency_ms=$latency"
  if ($latency -gt $BudgetMs) {
    throw "expected A KeyUp within ${BudgetMs}ms after $Mode, got ${latency}ms"
  }

  if ($Mode -eq 'panic' -and -not $proc.HasExited) {
    try { $proc.StandardInput.Close() } catch { }
  }
  $stdoutDrain = $proc.StandardOutput.ReadToEndAsync()
  $stderrDrain = $proc.StandardError.ReadToEndAsync()
  if (-not $proc.WaitForExit(20000)) {
    try { $proc.Kill() } catch { }
    throw "timed out waiting for child exit after $Mode"
  }
  $exitCode = $proc.ExitCode
  if ($ignoreCtrlC) {
    [ConsoleCtrl]::RemoveIgnoreHandler() | Out-Null
    $ignoreCtrlC = $false
  }

  $stdoutRemainder = $stdoutDrain.Result
  $stderr = $stderrDrain.Result
  if (-not [string]::IsNullOrWhiteSpace($stdoutRemainder)) {
    Write-Output "source_of_truth=daemon_stdout edge=$Mode after=$stdoutRemainder"
  }
  if (-not [string]::IsNullOrWhiteSpace($stderr)) {
    Write-Output "source_of_truth=daemon_stderr edge=$Mode after=$stderr"
  }
  if ($Mode -eq 'panic') {
    $releaseLine = Read-ReleasedKeyLine $logDir
    $panicLine = Read-SafetyLine $logDir 'panic'
    Write-Output "source_of_truth=daemon_log edge=panic after_exit=$exitCode after_release_line=$releaseLine"
    Write-Output "source_of_truth=daemon_log edge=panic after_panic_line=$panicLine"
    exit 0
  }
  $safetyLine = Read-SafetyLine $logDir $Mode
  Write-Output "source_of_truth=daemon_log edge=$Mode after_exit=$exitCode after_safety_line=$safetyLine"
  if ($Mode -ne 'panic' -and $exitCode -ne 0) {
    throw "expected daemon exit code 0 after $Mode, got $exitCode"
  }
  if ($safetyLine -notmatch '"released_keys":1') {
    throw "expected released_keys=1 in safety line: $safetyLine"
  }
  exit 0
} catch {
  Write-Error $_
  exit 1
} finally {
  if ($ignoreCtrlC) {
    [ConsoleCtrl]::RemoveIgnoreHandler() | Out-Null
  }
  if ($null -ne $proc -and -not $proc.HasExited) {
    try { $proc.Kill() } catch { }
  }
  [KeyboardHookRecorder]::Stop()
}
"#;

    #[test]
    fn powershell_script_contains_required_source_of_truth_markers() {
        println!(
            "source_of_truth=keyboard_hook_script edge=markers after_len={} budget_ms={SIGINT_RELEASE_BUDGET_MS}",
            POWERSHELL_HOOK_SCRIPT.len()
        );
        assert!(POWERSHELL_HOOK_SCRIPT.contains("SetWindowsHookEx"));
        assert!(POWERSHELL_HOOK_SCRIPT.contains("WH_KEYBOARD_LL"));
        assert!(POWERSHELL_HOOK_SCRIPT.contains("GenerateConsoleCtrlEvent"));
        assert!(POWERSHELL_HOOK_SCRIPT.contains("source_of_truth=keyboard_hook_timeline"));
        assert!(POWERSHELL_HOOK_SCRIPT.contains("source_of_truth=daemon_log"));
    }
}
