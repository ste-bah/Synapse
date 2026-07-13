# Supporting diagnostic only. Its output is supporting diagnostic evidence only.
# This script does not perform or accept Full State Verification (FSV). Under
# AGENTS.md D1, an agent must perform FSV manually
# through the strict production MCP client and independently read each physical
# Source of Truth before and after the trigger.
param(
    [string]$Bind = '127.0.0.1:7700',
    [string]$TokenPath = "$env:APPDATA\synapse\token.txt"
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Die($Message) { throw "[issue994-diagnostic] $Message" }

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { Die $Message }
}

function Get-WebResponseUtf8Content {
    param([Parameter(Mandatory = $true)]$Response)
    $streamProperty = $Response.PSObject.Properties['RawContentStream']
    if ($streamProperty -and $null -ne $streamProperty.Value) {
        $stream = $streamProperty.Value
        if ($stream.CanSeek) { $stream.Position = 0 }
        $reader = [System.IO.StreamReader]::new($stream, [System.Text.UTF8Encoding]::new($false, $true), $true, 4096, $true)
        try { return $reader.ReadToEnd() } finally {
            $reader.Dispose()
            if ($stream.CanSeek) { $stream.Position = 0 }
        }
    }
    if ($Response.Content -is [byte[]]) {
        return [System.Text.UTF8Encoding]::new($false, $true).GetString($Response.Content)
    }
    [string]$Response.Content
}

function Read-McpSseJsonResponse {
    param(
        [Parameter(Mandatory = $true)][string]$Content,
        [Parameter(Mandatory = $true)][string]$Operation,
        [int]$ExpectedId = 0
    )

    $trimmed = $Content.Trim()
    if ($trimmed.StartsWith('{')) {
        $message = $trimmed | ConvertFrom-Json
    } else {
        $normalized = ($Content -replace "`r`n", "`n") -replace "`r", "`n"
        $message = $null
        foreach ($frame in @($normalized -split "`n`n")) {
            $dataLines = @()
            foreach ($line in @($frame -split "`n")) {
                if ($line.StartsWith('data:')) { $dataLines += $line.Substring(5).TrimStart() }
            }
            $data = ($dataLines -join "`n").Trim()
            if ($data.StartsWith('{')) {
                $message = $data | ConvertFrom-Json
                break
            }
        }
        if ($null -eq $message) {
            $prefix = if ($Content.Length -gt 240) { $Content.Substring(0, 240) } else { $Content }
            Die "SSE_PARSE_FAILED operation=$Operation content_prefix=$prefix"
        }
    }

    if ($ExpectedId -ne 0 -and [int]$message.id -ne $ExpectedId) {
        Die "JSONRPC_ID_MISMATCH operation=$Operation expected=$ExpectedId actual=$($message.id)"
    }
    $errorProperty = $message.PSObject.Properties['error']
    if ($errorProperty -and $null -ne $errorProperty.Value) {
        $errorJson = $errorProperty.Value | ConvertTo-Json -Depth 20 -Compress
        Die "JSONRPC_ERROR operation=$Operation error=$errorJson"
    }
    $message
}

function Invoke-McpHttpPost {
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$Method,
        [Parameter(Mandatory = $true)]$Params,
        [int]$Id = 0,
        [string]$SessionId,
        [int]$TimeoutSec = 30
    )

    $headers = @{
        Authorization = "Bearer $Token"
        Accept = 'application/json, text/event-stream'
    }
    if (-not [string]::IsNullOrWhiteSpace($SessionId)) { $headers['Mcp-Session-Id'] = $SessionId }
    $request = [ordered]@{ jsonrpc = '2.0'; method = $Method; params = $Params }
    if ($Id -ne 0) { $request['id'] = $Id }
    $body = $request | ConvertTo-Json -Depth 100 -Compress
    $response = Invoke-WebRequest -Uri "http://$Bind/mcp" -Method Post -Headers $headers -ContentType 'application/json' -Body $body -TimeoutSec $TimeoutSec -UseBasicParsing -ErrorAction Stop
    [pscustomobject]@{
        Content = Get-WebResponseUtf8Content -Response $response
        Headers = $response.Headers
        StatusCode = $response.StatusCode
    }
}

function Open-McpSession {
    param([string]$Bind, [string]$Token, [string]$Name)
    $initParams = [ordered]@{
        protocolVersion = '2025-06-18'
        capabilities = @{}
        clientInfo = [ordered]@{ name = $Name; version = '1' }
    }
    $initResponse = Invoke-McpHttpPost -Bind $Bind -Token $Token -Method 'initialize' -Params $initParams -Id 1
    $sessionId = @($initResponse.Headers['Mcp-Session-Id'])[0]
    Assert-True (-not [string]::IsNullOrWhiteSpace($sessionId)) "initialize did not return Mcp-Session-Id for $Name"
    $initMessage = Read-McpSseJsonResponse -Content $initResponse.Content -Operation 'initialize' -ExpectedId 1
    Assert-True ($null -ne $initMessage.result.capabilities) "initialize response missing capabilities for $Name"
    Invoke-McpHttpPost -Bind $Bind -Token $Token -SessionId $sessionId -Method 'notifications/initialized' -Params @{} | Out-Null
    $sessionId
}

function Convert-McpToolResult {
    param([Parameter(Mandatory = $true)]$Message, [Parameter(Mandatory = $true)][string]$ToolName)
    $isErrorProperty = $Message.result.PSObject.Properties['isError']
    if ($isErrorProperty -and $isErrorProperty.Value -eq $true) {
        $errorText = @($Message.result.content | Where-Object { [string]$_.type -eq 'text' } | Select-Object -First 1).text
        Die "TOOL_CALL_ERROR tool=$ToolName error=$errorText"
    }
    $structured = $Message.result.PSObject.Properties['structuredContent']
    if ($structured -and $null -ne $structured.Value) { return $structured.Value }
    $text = @($Message.result.content | Where-Object { [string]$_.type -eq 'text' } | Select-Object -First 1).text
    if ([string]::IsNullOrWhiteSpace($text)) { return $null }
    $text | ConvertFrom-Json
}

function Invoke-McpTool {
    param(
        [string]$Bind,
        [string]$Token,
        [string]$SessionId,
        [ref]$NextId,
        [string]$Name,
        $Arguments,
        [int]$TimeoutSec = 60
    )
    $requestId = $NextId.Value
    $NextId.Value = $NextId.Value + 1
    $params = @{ name = $Name; arguments = $Arguments }
    $response = Invoke-McpHttpPost -Bind $Bind -Token $Token -SessionId $SessionId -Method 'tools/call' -Params $params -Id $requestId -TimeoutSec $TimeoutSec
    $message = Read-McpSseJsonResponse -Content $response.Content -Operation "tools/call $Name" -ExpectedId $requestId
    Convert-McpToolResult -Message $message -ToolName $Name
}

function Start-McpToolCallAsync {
    param(
        [string]$Bind,
        [string]$Token,
        [string]$SessionId,
        [ref]$NextId,
        [string]$Name,
        $Arguments,
        [int]$TimeoutSec = 120
    )
    Add-Type -AssemblyName System.Net.Http
    $requestId = $NextId.Value
    $NextId.Value = $NextId.Value + 1
    $request = [ordered]@{
        jsonrpc = '2.0'
        id = $requestId
        method = 'tools/call'
        params = @{ name = $Name; arguments = $Arguments }
    }
    $body = $request | ConvertTo-Json -Depth 100 -Compress
    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromSeconds($TimeoutSec)
    $message = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://$Bind/mcp")
    $message.Headers.TryAddWithoutValidation('Authorization', "Bearer $Token") | Out-Null
    $message.Headers.TryAddWithoutValidation('Accept', 'application/json, text/event-stream') | Out-Null
    $message.Headers.TryAddWithoutValidation('Mcp-Session-Id', $SessionId) | Out-Null
    $message.Content = [System.Net.Http.StringContent]::new($body, [System.Text.Encoding]::UTF8, 'application/json')
    [pscustomobject]@{ Client = $client; Request = $message; Task = $client.SendAsync($message); RequestId = $requestId; Name = $Name }
}

function Receive-McpToolCallAsync {
    param([Parameter(Mandatory = $true)]$Call)
    try {
        $response = $Call.Task.GetAwaiter().GetResult()
        $content = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        if (-not $response.IsSuccessStatusCode) {
            Die "ASYNC_HTTP_ERROR tool=$($Call.Name) status=$([int]$response.StatusCode) body=$content"
        }
        $message = Read-McpSseJsonResponse -Content $content -Operation "async tools/call $($Call.Name)" -ExpectedId $Call.RequestId
        Convert-McpToolResult -Message $message -ToolName $Call.Name
    } finally {
        if ($Call.PSObject.Properties['Request']) { $Call.Request.Dispose() }
        if ($Call.PSObject.Properties['Client']) { $Call.Client.Dispose() }
    }
}

function Close-McpSession {
    param([string]$Bind, [string]$Token, [string]$SessionId)
    $headers = @{ Authorization = "Bearer $Token"; Accept = 'application/json, text/event-stream'; 'Mcp-Session-Id' = $SessionId }
    try { Invoke-WebRequest -Uri "http://$Bind/mcp" -Method Delete -Headers $headers -TimeoutSec 5 -UseBasicParsing -ErrorAction Stop | Out-Null } catch {}
}

function Get-JsonText {
    param([Parameter(Mandatory = $true)]$Value)
    $Value | ConvertTo-Json -Depth 100 -Compress
}

function Assert-JsonContainsOnlyOwnMarker {
    param($Value, [string]$OwnMarker, [string]$OtherMarker, [string]$Label)
    $json = Get-JsonText $Value
    Assert-True ($json.Contains($OwnMarker)) "$Label missing own marker $OwnMarker"
    Assert-True (-not $json.Contains($OtherMarker)) "$Label leaked other marker $OtherMarker"
}

$user32 = @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Issue994User32 {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern void SwitchToThisWindow(IntPtr hWnd, bool fAltTab);
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT lpPoint);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
}
"@
Add-Type $user32 -ErrorAction SilentlyContinue

function Get-PhysicalSample {
    param([string]$Label)
    $point = [Issue994User32+POINT]::new()
    [Issue994User32]::GetCursorPos([ref]$point) | Out-Null
    $hwnd = [Issue994User32]::GetForegroundWindow()
    [uint32]$windowProcessId = 0
    [Issue994User32]::GetWindowThreadProcessId($hwnd, [ref]$windowProcessId) | Out-Null
    $titleBuilder = [System.Text.StringBuilder]::new(512)
    [Issue994User32]::GetWindowText($hwnd, $titleBuilder, $titleBuilder.Capacity) | Out-Null
    $proc = if ($windowProcessId -ne 0) { Get-Process -Id ([int]$windowProcessId) -ErrorAction SilentlyContinue } else { $null }
    [pscustomobject]@{
        label = $Label
        foreground_hwnd = [int64]$hwnd
        pid = [int]$windowProcessId
        process_name = if ($proc) { $proc.ProcessName } else { $null }
        title = $titleBuilder.ToString()
        cursor_x = $point.X
        cursor_y = $point.Y
    }
}

function Set-DriverForegroundWindow {
    param([Parameter(Mandatory = $true)][int64]$Hwnd)
    $ptr = [IntPtr]$Hwnd
    [Issue994User32]::ShowWindow($ptr, 9) | Out-Null
    try {
        [Issue994User32]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
        [Issue994User32]::SwitchToThisWindow($ptr, $true)
        [Issue994User32]::BringWindowToTop($ptr) | Out-Null
        [Issue994User32]::SetForegroundWindow($ptr) | Out-Null
    } finally {
        [Issue994User32]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
    }
}

function Invoke-ForegroundDriverSamples {
    param(
        [Parameter(Mandatory = $true)][int64]$HwndOne,
        [Parameter(Mandatory = $true)][int64]$HwndTwo,
        [Parameter(Mandatory = $true)][string]$Phase,
        [int]$StartIndex = 0,
        [int]$Count = 14
    )
    $samples = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $Count; $i++) {
        $index = $StartIndex + $i
        $target = if (($index % 2) -eq 0) { $HwndOne } else { $HwndTwo }
        Set-DriverForegroundWindow -Hwnd $target
        [Issue994User32]::SetCursorPos((120 + (($index * 37) % 900)), (180 + (($index * 29) % 650))) | Out-Null
        Start-Sleep -Milliseconds 110
        $samples.Add((Get-PhysicalSample -Label "$Phase-$index"))
    }
    @($samples)
}

if (Test-Path -LiteralPath $TokenPath) {
    $token = (Get-Content -LiteralPath $TokenPath -Raw).Trim()
} elseif (-not [string]::IsNullOrWhiteSpace($env:SYNAPSE_BEARER_TOKEN)) {
    $token = $env:SYNAPSE_BEARER_TOKEN
} else {
    Die "missing Synapse bearer token"
}

$nextA = 10
$nextB = 1000
$nextO = 2000
$sessionA = $null
$sessionB = $null
$observer = $null
$notepad = $null
$startedNotepad = $null
$targetA = $null
$targetB = $null

try {
    $marker = "issue994-human-active-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
    $markerA = "$marker-A"
    $markerB = "$marker-B"

    $observer = Open-McpSession -Bind $Bind -Token $token -Name 'issue994-diagnostic-observer'
    $sessionA = Open-McpSession -Bind $Bind -Token $token -Name 'issue994-diagnostic-session-a'
    $sessionB = Open-McpSession -Bind $Bind -Token $token -Name 'issue994-diagnostic-session-b'

    $health = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'health' -Arguments @{} -TimeoutSec 30
    Assert-True ([bool]$health.ok) 'health not ok'

    $windows = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'window_list' -Arguments @{ process_name_contains = 'chrome'; exclude_minimized = $false } -TimeoutSec 30
    $chrome = @($windows.windows | Where-Object { [bool]$_.is_chromium -and -not [bool]$_.is_foreground } | Sort-Object { [int]$_.window_bounds.x } | Select-Object -First 1)[0]
    Assert-True ($null -ne $chrome) 'no non-foreground Chrome window found'
    $chromeHwnd = [int64]$chrome.hwnd

    $code = Get-Process -Name Code -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    Assert-True ($null -ne $code) 'no VS Code window found for human-active foreground driver'

    $notepad = Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($null -eq $notepad) {
        $startedNotepad = Start-Process notepad.exe -PassThru
    }
    $deadline = (Get-Date).AddSeconds(20)
    do {
        Start-Sleep -Milliseconds 100
        if ($null -ne $startedNotepad) {
            try { $startedNotepad.Refresh() } catch {}
            $notepad = Get-Process -Id $startedNotepad.Id -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        }
        if ($null -eq $notepad) {
            $notepad = Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        }
    } while (($null -eq $notepad -or $notepad.MainWindowHandle -eq 0) -and (Get-Date) -lt $deadline)
    if ($null -eq $notepad -or $notepad.MainWindowHandle -eq 0) {
        $notepad = Get-Process | Where-Object {
            $_.MainWindowHandle -ne 0 -and
            $_.Id -ne $code.Id -and
            $_.ProcessName -notlike '*chrome*' -and
            $_.ProcessName -ne 'Code'
        } | Select-Object -First 1
    }
    Assert-True ($null -ne $notepad -and $notepad.MainWindowHandle -ne 0) 'no second visible foreground-driver window found'

    [Issue994User32]::ShowWindow([IntPtr]$notepad.MainWindowHandle, 9) | Out-Null
    $baselineWin32 = Get-PhysicalSample -Label 'baseline-before-targets'

    $openA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'cdp_open_tab' -Arguments @{ window_hwnd = $chromeHwnd; url = 'about:blank' } -TimeoutSec 30
    $openB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'cdp_open_tab' -Arguments @{ window_hwnd = $chromeHwnd; url = 'about:blank' } -TimeoutSec 30
    $targetA = [string]$openA.cdp_target_id
    $targetB = [string]$openB.cdp_target_id
    Assert-True (-not [string]::IsNullOrWhiteSpace($targetA)) 'session A did not receive CDP target'
    Assert-True (-not [string]::IsNullOrWhiteSpace($targetB)) 'session B did not receive CDP target'
    Assert-True ($targetA -ne $targetB) 'sessions received the same CDP target'
    Assert-True ([int64]$openA.human_os_foreground_before_hwnd -eq [int64]$openA.human_os_foreground_after_hwnd) 'session A open changed human foreground'
    Assert-True ([int64]$openB.human_os_foreground_before_hwnd -eq [int64]$openB.human_os_foreground_after_hwnd) 'session B open changed human foreground'

    $claimA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'target_claim' -Arguments @{} -TimeoutSec 30
    $claimB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'target_claim' -Arguments @{} -TimeoutSec 30

    $htmlA = "<!doctype html><meta charset='utf-8'><title>$markerA</title><h1 id='marker'>$markerA</h1><input id='value' value='before-a'><button id='go' onclick=`"document.getElementById('status').textContent=document.getElementById('value').value`">Apply A</button><p id='status'>ready-a</p>"
    $htmlB = "<!doctype html><meta charset='utf-8'><title>$markerB</title><h1 id='marker'>$markerB</h1><input id='value' value='before-b'><button id='go' onclick=`"document.getElementById('status').textContent=document.getElementById('value').value`">Apply B</button><p id='status'>ready-b</p>"
    $setContentA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'browser_set_content' -Arguments @{ html = $htmlA; wait_timeout_ms = 5000 } -TimeoutSec 30
    $setContentB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'browser_set_content' -Arguments @{ html = $htmlB; wait_timeout_ms = 5000 } -TimeoutSec 30
    Assert-True (-not [bool]$setContentA.required_foreground) 'session A set_content required foreground'
    Assert-True (-not [bool]$setContentB.required_foreground) 'session B set_content required foreground'

    $driverSamples = @()
    Start-Sleep -Milliseconds 250
    $setA = Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'target_act' -Arguments @{ verb = 'set_field'; selector = '#value'; text = "$markerA-value"; wait_timeout_ms = 5000 } -TimeoutSec 60
    $setB = Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'target_act' -Arguments @{ verb = 'set_field'; selector = '#value'; text = "$markerB-value"; wait_timeout_ms = 5000 } -TimeoutSec 60
    $driverSamples += Invoke-ForegroundDriverSamples -HwndOne ([int64]$notepad.MainWindowHandle) -HwndTwo ([int64]$code.MainWindowHandle) -Phase 'set-field' -StartIndex 0 -Count 14
    $setResultA = Receive-McpToolCallAsync -Call $setA
    $setResultB = Receive-McpToolCallAsync -Call $setB
    $pressA = Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'target_act' -Arguments @{ verb = 'press'; selector = '#go'; wait_timeout_ms = 5000 } -TimeoutSec 60
    $pressB = Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'target_act' -Arguments @{ verb = 'press'; selector = '#go'; wait_timeout_ms = 5000 } -TimeoutSec 60
    $driverSamples += Invoke-ForegroundDriverSamples -HwndOne ([int64]$notepad.MainWindowHandle) -HwndTwo ([int64]$code.MainWindowHandle) -Phase 'press' -StartIndex 14 -Count 14
    $pressResultA = Receive-McpToolCallAsync -Call $pressA
    $pressResultB = Receive-McpToolCallAsync -Call $pressB

    Assert-True ($driverSamples.Count -ge 10) 'foreground driver did not produce enough samples'
    $sampleForegrounds = @($driverSamples | Select-Object -ExpandProperty foreground_hwnd -Unique)
    $sampleCursorPairs = @($driverSamples | ForEach-Object { "$($_.cursor_x),$($_.cursor_y)" } | Select-Object -Unique)
    Assert-True ($sampleForegrounds.Count -ge 2) 'foreground driver did not switch foreground windows'
    Assert-True ($sampleCursorPairs.Count -ge 5) 'foreground driver did not move cursor enough'

    foreach ($result in @($setResultA, $setResultB, $pressResultA, $pressResultB)) {
        Assert-True (-not [bool]$result.result.required_foreground) 'target_act unexpectedly required foreground'
        Assert-True ([string]$result.status -eq 'ok') 'target_act did not return ok'
    }

    $readA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'target_act' -Arguments @{ verb = 'read'; wait_timeout_ms = 5000 } -TimeoutSec 30
    $readB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'target_act' -Arguments @{ verb = 'read'; wait_timeout_ms = 5000 } -TimeoutSec 30
    Assert-JsonContainsOnlyOwnMarker -Value $readA -OwnMarker $markerA -OtherMarker $markerB -Label 'session A readback'
    Assert-JsonContainsOnlyOwnMarker -Value $readB -OwnMarker $markerB -OtherMarker $markerA -Label 'session B readback'

    $sessionListDuring = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_list' -Arguments @{ view = 'full'; live_only = $true; include_attached_agent_rows = $true; limit = 20 } -TimeoutSec 30
    Assert-True ([int]$sessionListDuring.foreground_lane_capacity.active_foreground_lane_count -ge 2) 'session_list did not report active lanes during run'
    Assert-True ([int]$sessionListDuring.foreground_lane_capacity.claimed_target_lane_count -ge 2) 'session_list did not report claimed lanes during run'

    $claimsDuring = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 30
    Assert-True ([int]$claimsDuring.claim_count -ge 2) 'target_claim_status did not report both claims'
    $leaseDuring = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'control_lease_status' -Arguments @{} -TimeoutSec 30
    Assert-True (-not [bool]$leaseDuring.held) 'unexpected real foreground lease during normal target work'

    $profileA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'tool_profile_status' -Arguments @{} -TimeoutSec 30
    $profileB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'tool_profile_status' -Arguments @{} -TimeoutSec 30
    Assert-True ([bool]$profileA.snapshot.foreground_capability.profile_preserves_capability) 'profile A does not preserve foreground capability'
    Assert-True ([bool]$profileB.snapshot.foreground_capability.profile_preserves_capability) 'profile B does not preserve foreground capability'

    $finalWin32BeforeCleanup = Get-PhysicalSample -Label 'after-actions-before-cleanup'

    $closeA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'cdp_close_tab' -Arguments @{ cdp_target_id = $targetA } -TimeoutSec 30
    $closeB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'cdp_close_tab' -Arguments @{ cdp_target_id = $targetB } -TimeoutSec 30
    Assert-True ([bool]$closeA.closed) 'session A tab did not close'
    Assert-True ([bool]$closeB.closed) 'session B tab did not close'

    $endA = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'session_end' -Arguments @{} -TimeoutSec 60
    $endB = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'session_end' -Arguments @{} -TimeoutSec 60
    Assert-True ([int]$endA.report.failure_count -eq 0) 'session A cleanup had failures'
    Assert-True ([int]$endB.report.failure_count -eq 0) 'session B cleanup had failures'
    Close-McpSession -Bind $Bind -Token $token -SessionId $sessionA
    Close-McpSession -Bind $Bind -Token $token -SessionId $sessionB

    $sessionListFinal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_list' -Arguments @{ view = 'full'; live_only = $true; include_attached_agent_rows = $true; limit = 20 } -TimeoutSec 30
    $claimsFinal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 30
    $leaseFinal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'control_lease_status' -Arguments @{} -TimeoutSec 30
    Assert-True ([int]$sessionListFinal.foreground_lane_capacity.claimed_target_lane_count -eq 0) 'claimed lanes remained after cleanup'
    Assert-True ([int]$claimsFinal.claim_count -eq 0) 'claims remained after cleanup'
    Assert-True (-not [bool]$leaseFinal.held) 'lease remained after cleanup'

    $summary = [ordered]@{
        marker = $marker
        health = @{
            ok = [bool]$health.ok
            pid = [int]$health.pid
            tool_count = [int]$health.tool_count
            chrome_bridge = [string]$health.subsystems.chrome_bridge.status
        }
        sessions = @{
            observer = $observer
            a = $sessionA
            b = $sessionB
        }
        chrome = @{
            hwnd = $chromeHwnd
            title = [string]$chrome.window_title
            open_a_target = $targetA
            open_b_target = $targetB
            open_a_human_fg_before = [int64]$openA.human_os_foreground_before_hwnd
            open_a_human_fg_after = [int64]$openA.human_os_foreground_after_hwnd
            open_b_human_fg_before = [int64]$openB.human_os_foreground_before_hwnd
            open_b_human_fg_after = [int64]$openB.human_os_foreground_after_hwnd
        }
        foreground_driver = @{
            baseline = $baselineWin32
            final_before_cleanup = $finalWin32BeforeCleanup
            sample_count = $driverSamples.Count
            distinct_foreground_hwnds = @($sampleForegrounds)
            distinct_cursor_positions = $sampleCursorPairs.Count
            first_samples = @($driverSamples | Select-Object -First 6)
        }
        actions = @{
            set_a_status = [string]$setResultA.status
            set_b_status = [string]$setResultB.status
            press_a_status = [string]$pressResultA.status
            press_b_status = [string]$pressResultB.status
            set_a_required_foreground = [bool]$setResultA.result.required_foreground
            set_b_required_foreground = [bool]$setResultB.result.required_foreground
            press_a_required_foreground = [bool]$pressResultA.result.required_foreground
            press_b_required_foreground = [bool]$pressResultB.result.required_foreground
        }
        readback = @{
            session_a_contains = $markerA
            session_b_contains = $markerB
            session_a_other_marker_absent = $true
            session_b_other_marker_absent = $true
        }
        lanes_during = @{
            active_foreground_lane_count = [int]$sessionListDuring.foreground_lane_capacity.active_foreground_lane_count
            claimed_target_lane_count = [int]$sessionListDuring.foreground_lane_capacity.claimed_target_lane_count
            explicit_real_foreground_lease_count = [int]$sessionListDuring.foreground_lane_capacity.explicit_real_foreground_lease_count
            capacity_exhausted = [bool]$sessionListDuring.foreground_lane_capacity.capacity_exhausted
            claim_count = [int]$claimsDuring.claim_count
            lease_held = [bool]$leaseDuring.held
        }
        cleanup = @{
            close_a = [bool]$closeA.closed
            close_b = [bool]$closeB.closed
            end_a_failures = [int]$endA.report.failure_count
            end_b_failures = [int]$endB.report.failure_count
            final_claim_count = [int]$claimsFinal.claim_count
            final_claimed_lanes = [int]$sessionListFinal.foreground_lane_capacity.claimed_target_lane_count
            final_active_lanes = [int]$sessionListFinal.foreground_lane_capacity.active_foreground_lane_count
            final_lease_held = [bool]$leaseFinal.held
        }
    }
    $summary | ConvertTo-Json -Depth 100
} finally {
    if ($null -ne $targetA -and $null -ne $sessionA) {
        try { Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'cdp_close_tab' -Arguments @{ cdp_target_id = $targetA } -TimeoutSec 10 | Out-Null } catch {}
    }
    if ($null -ne $targetB -and $null -ne $sessionB) {
        try { Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'cdp_close_tab' -Arguments @{ cdp_target_id = $targetB } -TimeoutSec 10 | Out-Null } catch {}
    }
    if ($null -ne $sessionA) { try { Close-McpSession -Bind $Bind -Token $token -SessionId $sessionA } catch {} }
    if ($null -ne $sessionB) { try { Close-McpSession -Bind $Bind -Token $token -SessionId $sessionB } catch {} }
    if ($null -ne $observer) {
        try { Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_end' -Arguments @{} -TimeoutSec 20 | Out-Null } catch {}
        try { Close-McpSession -Bind $Bind -Token $token -SessionId $observer } catch {}
    }
    if ($null -ne $startedNotepad -and -not $startedNotepad.HasExited) {
        try { $startedNotepad.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 300 } catch {}
        if (-not $startedNotepad.HasExited) { try { $startedNotepad.Kill() } catch {} }
    }
}
