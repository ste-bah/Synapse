# Supporting diagnostic only. Its output is supporting diagnostic evidence only.
# This script does not perform or accept Full State Verification (FSV). Under
# AGENTS.md D1, an agent must perform FSV manually
# through the strict production MCP client and independently read each physical
# Source of Truth before and after the trigger.
param(
    [int]$Count = 50,
    [int]$BatchSize = 10,
    [string]$Bind = '127.0.0.1:7700',
    [string]$TokenPath = "$env:APPDATA\synapse\token.txt",
    [string]$OutputDir = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Die($Message) { throw "[issue1220-diagnostic] $Message" }

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

function Invoke-McpToolMaybe {
    param(
        [string]$Bind,
        [string]$Token,
        [string]$SessionId,
        [ref]$NextId,
        [string]$Name,
        $Arguments,
        [int]$TimeoutSec = 60
    )
    try {
        [pscustomobject]@{
            ok = $true
            value = Invoke-McpTool -Bind $Bind -Token $Token -SessionId $SessionId -NextId $NextId -Name $Name -Arguments $Arguments -TimeoutSec $TimeoutSec
            error = $null
        }
    } catch {
        [pscustomobject]@{ ok = $false; value = $null; error = $_.Exception.Message }
    }
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

function Get-PropValue {
    param($Value, [string]$Name)
    if ($null -eq $Value) { return $null }
    $prop = $Value.PSObject.Properties[$Name]
    if ($prop) { return $prop.Value }
    $null
}

function Get-RequiredForeground {
    param($Value)
    $top = Get-PropValue $Value 'required_foreground'
    if ($null -ne $top -and [bool]$top) { return $true }
    $result = Get-PropValue $Value 'result'
    $nested = Get-PropValue $result 'required_foreground'
    if ($null -ne $nested -and [bool]$nested) { return $true }
    $routing = Get-PropValue $Value 'routing'
    $routingRequired = Get-PropValue $routing 'required_foreground'
    if ($null -ne $routingRequired -and [bool]$routingRequired) { return $true }
    $false
}

function Assert-RoutineActionOk {
    param($Value, [string]$Label)
    $ok = Get-PropValue $Value 'ok'
    $status = [string](Get-PropValue $Value 'status')
    if ($null -ne $ok) { Assert-True ([bool]$ok) "$Label returned ok=false" }
    if (-not [string]::IsNullOrWhiteSpace($status)) {
        Assert-True (@('ok', 'verified_state', 'completed') -contains $status) "$Label returned unexpected status $status"
    }
    Assert-True (-not (Get-RequiredForeground $Value)) "$Label unexpectedly required real foreground"
}

function Assert-JsonContainsOnlyOwnSample {
    param($Value, [string]$OwnMarker, [string[]]$OtherMarkers, [string]$Label)
    $json = Get-JsonText $Value
    Assert-True ($json.Contains($OwnMarker)) "$Label missing own marker $OwnMarker"
    foreach ($other in $OtherMarkers) {
        if (-not [string]::IsNullOrWhiteSpace($other)) {
            Assert-True (-not $json.Contains($other)) "$Label leaked other marker $other"
        }
    }
}

function Quote-PsSingle {
    param([string]$Value)
    "'" + ($Value -replace "'", "''") + "'"
}

$user32 = @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Issue1220User32 {
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
    $point = [Issue1220User32+POINT]::new()
    [Issue1220User32]::GetCursorPos([ref]$point) | Out-Null
    $hwnd = [Issue1220User32]::GetForegroundWindow()
    [uint32]$windowProcessId = 0
    [Issue1220User32]::GetWindowThreadProcessId($hwnd, [ref]$windowProcessId) | Out-Null
    $titleBuilder = [System.Text.StringBuilder]::new(512)
    [Issue1220User32]::GetWindowText($hwnd, $titleBuilder, $titleBuilder.Capacity) | Out-Null
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
    [Issue1220User32]::ShowWindow($ptr, 9) | Out-Null
    try {
        [Issue1220User32]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
        [Issue1220User32]::SwitchToThisWindow($ptr, $true)
        [Issue1220User32]::BringWindowToTop($ptr) | Out-Null
        [Issue1220User32]::SetForegroundWindow($ptr) | Out-Null
    } finally {
        [Issue1220User32]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
    }
}

function Invoke-ForegroundDriverSamples {
    param(
        [Parameter(Mandatory = $true)][int64]$HwndOne,
        [Parameter(Mandatory = $true)][int64]$HwndTwo,
        [Parameter(Mandatory = $true)][string]$Phase,
        [int]$StartIndex = 0,
        [int]$Count = 6
    )
    $samples = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $Count; $i++) {
        $index = $StartIndex + $i
        $target = if (($index % 2) -eq 0) { $HwndOne } else { $HwndTwo }
        Set-DriverForegroundWindow -Hwnd $target
        [Issue1220User32]::SetCursorPos((120 + (($index * 37) % 900)), (180 + (($index * 29) % 650))) | Out-Null
        Start-Sleep -Milliseconds 100
        $samples.Add((Get-PhysicalSample -Label "$Phase-$index"))
    }
    @($samples)
}

Assert-True ($Count -ge 2) 'Count must be at least 2'
Assert-True ($BatchSize -ge 1) 'BatchSize must be at least 1'

if (Test-Path -LiteralPath $TokenPath) {
    $token = (Get-Content -LiteralPath $TokenPath -Raw).Trim()
} elseif (-not [string]::IsNullOrWhiteSpace($env:SYNAPSE_BEARER_TOKEN)) {
    $token = $env:SYNAPSE_BEARER_TOKEN
} else {
    Die "missing Synapse bearer token"
}

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path (Join-Path (Get-Location).Path 'diagnostic-tmp') "issue-1220-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
}
$artifactRoot = (New-Item -ItemType Directory -Force -Path $OutputDir).FullName

$observer = $null
$lanes = [System.Collections.Generic.List[object]]::new()
$nextO = 20000
$notepad = $null
$startedNotepad = $null
$driverSampleIndex = 0

try {
    $runMarker = "issue1220-lanes-$(Get-Date -Format 'yyyyMMdd-HHmmss')"

    $code = Get-Process -Name Code -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    Assert-True ($null -ne $code) 'no VS Code window found for foreground driver'
    $notepad = Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($null -eq $notepad) { $startedNotepad = Start-Process notepad.exe -PassThru }
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
            $_.MainWindowHandle -ne 0 -and $_.Id -ne $code.Id -and $_.ProcessName -notlike '*chrome*' -and $_.ProcessName -ne 'Code'
        } | Select-Object -First 1
    }
    Assert-True ($null -ne $notepad -and $notepad.MainWindowHandle -ne 0) 'no second foreground-driver window found'
    Set-DriverForegroundWindow -Hwnd ([int64]$notepad.MainWindowHandle)

    $observer = Open-McpSession -Bind $Bind -Token $token -Name 'issue1220-diagnostic-observer'
    $health = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'health' -Arguments @{} -TimeoutSec 30
    Assert-True ([bool]$health.ok) 'health not ok'

    $windows = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'window_list' -Arguments @{ process_name_contains = 'chrome'; exclude_minimized = $false } -TimeoutSec 30
    $chrome = @($windows.windows | Where-Object { [bool]$_.is_chromium -and -not [bool]$_.is_foreground } | Sort-Object { [int]$_.window_bounds.x } | Select-Object -First 1)[0]
    Assert-True ($null -ne $chrome) 'no non-foreground Chrome window found'
    $chromeHwnd = [int64]$chrome.hwnd
    $baselineWin32 = Get-PhysicalSample -Label 'baseline-before-lanes'

    for ($i = 1; $i -le $Count; $i++) {
        $sessionName = 'issue1220-diagnostic-lane-{0:d2}' -f $i
        $sessionId = Open-McpSession -Bind $Bind -Token $token -Name $sessionName
        $nextId = 1000 + ($i * 100)
        $open = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'cdp_open_tab' -Arguments @{ window_hwnd = $chromeHwnd; url = 'about:blank' } -TimeoutSec 45
        $targetId = [string]$open.cdp_target_id
        Assert-True (-not [string]::IsNullOrWhiteSpace($targetId)) "lane $i did not receive a CDP target"
        Assert-True ([int64]$open.human_os_foreground_before_hwnd -eq [int64]$open.human_os_foreground_after_hwnd) "lane $i cdp_open_tab changed human foreground"
        $claim = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'target_claim' -Arguments @{ ttl_ms = 600000 } -TimeoutSec 30
        $marker = '{0}-{1:d2}' -f $runMarker, $i
        $html = "<!doctype html><meta charset='utf-8'><title>$marker</title><h1 id='marker'>$marker</h1><input id='value' value='before-$i'><button id='go' onclick=`"document.getElementById('status').textContent=document.getElementById('value').value`">Apply</button><p id='status'>ready-$i</p>"
        $setContent = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'browser_set_content' -Arguments @{ html = $html; wait_timeout_ms = 5000 } -TimeoutSec 45
        Assert-True (-not (Get-RequiredForeground $setContent)) "lane $i set_content required foreground"
        $lanes.Add([pscustomobject]@{
            index = $i
            name = $sessionName
            session_id = $sessionId
            next_id = $nextId
            target_id = $targetId
            marker = $marker
            expected_marker = $marker
            disconnected = $false
            navigate_marker = $null
        })
    }

    $sessionListAtCapacity = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_list' -Arguments @{ view = 'full'; live_only = $true; include_attached_agent_rows = $true; limit = ($Count + 10) } -TimeoutSec 60
    Assert-True ([int]$sessionListAtCapacity.foreground_lane_capacity.active_foreground_lane_count -ge $Count) 'session_list did not report all active foreground lanes'
    Assert-True ([int]$sessionListAtCapacity.foreground_lane_capacity.claimed_target_lane_count -ge $Count) 'session_list did not report all claimed lanes'
    $claimsAtCapacity = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 60
    Assert-True ([int]$claimsAtCapacity.claim_count -ge $Count) 'target_claim_status did not report all claims'
    $leaseAtCapacity = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'control_lease_status' -Arguments @{} -TimeoutSec 30
    Assert-True (-not [bool]$leaseAtCapacity.held) 'unexpected foreground lease before routine actions'

    $cross = Invoke-McpToolMaybe -Bind $Bind -Token $token -SessionId $lanes[0].session_id -NextId ([ref]$lanes[0].next_id) -Name 'browser_evaluate' -Arguments @{ cdp_target_id = $lanes[1].target_id; window_hwnd = $chromeHwnd; expression = 'document.title'; return_by_value = $true } -TimeoutSec 30
    $crossStatus = if ($cross.ok) { [string](Get-PropValue $cross.value 'status') } else { 'tool_error' }
    $crossDenied = (-not $cross.ok) -or ($cross.value -and (Get-PropValue $cross.value 'ok') -eq $false) -or (@('refused', 'error') -contains $crossStatus)
    Assert-True ([bool]$crossDenied) 'cross-target browser_evaluate was not denied'

    $profileSet = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[2].session_id -NextId ([ref]$lanes[2].next_id) -Name 'tool_profile_set' -Arguments @{ profile = 'browser_control'; reason = 'issue1220 stale profile refresh edge' } -TimeoutSec 30
    $profileRefresh = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[2].session_id -NextId ([ref]$lanes[2].next_id) -Name 'tool_profile_status' -Arguments @{} -TimeoutSec 30
    Assert-True ([bool]$profileRefresh.snapshot.foreground_capability.profile_preserves_capability) 'browser_control profile did not preserve capability'
    Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[2].session_id -NextId ([ref]$lanes[2].next_id) -Name 'tool_profile_set' -Arguments @{ profile = 'normal_agent'; reason = 'issue1220 restore normal profile after refresh edge' } -TimeoutSec 30 | Out-Null

    $driverSamples = @()
    $setResults = [System.Collections.Generic.List[object]]::new()
    for ($offset = 0; $offset -lt $lanes.Count; $offset += $BatchSize) {
        $batch = @($lanes | Select-Object -Skip $offset -First $BatchSize)
        $calls = @()
        foreach ($lane in $batch) {
            $calls += Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_act' -Arguments @{ verb = 'set_field'; selector = '#value'; text = "$($lane.marker)-value"; wait_timeout_ms = 5000 } -TimeoutSec 90
        }
        $driverSamples += Invoke-ForegroundDriverSamples -HwndOne ([int64]$notepad.MainWindowHandle) -HwndTwo ([int64]$code.MainWindowHandle) -Phase 'set-field' -StartIndex $driverSampleIndex -Count 6
        $driverSampleIndex += 6
        foreach ($call in $calls) {
            $result = Receive-McpToolCallAsync -Call $call
            Assert-RoutineActionOk -Value $result -Label "set_field batch offset $offset"
            $setResults.Add($result)
        }
    }

    $pressResults = [System.Collections.Generic.List[object]]::new()
    for ($offset = 0; $offset -lt $lanes.Count; $offset += $BatchSize) {
        $batch = @($lanes | Select-Object -Skip $offset -First $BatchSize)
        $calls = @()
        foreach ($lane in $batch) {
            $calls += Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_act' -Arguments @{ verb = 'press'; selector = '#go'; wait_timeout_ms = 5000 } -TimeoutSec 90
        }
        $driverSamples += Invoke-ForegroundDriverSamples -HwndOne ([int64]$notepad.MainWindowHandle) -HwndTwo ([int64]$code.MainWindowHandle) -Phase 'press' -StartIndex $driverSampleIndex -Count 6
        $driverSampleIndex += 6
        foreach ($call in $calls) {
            $result = Receive-McpToolCallAsync -Call $call
            Assert-RoutineActionOk -Value $result -Label "press batch offset $offset"
            $pressResults.Add($result)
        }
    }

    foreach ($lane in $lanes) {
        Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_claim' -Arguments @{ ttl_ms = 600000 } -TimeoutSec 30 | Out-Null
    }
    $claimsAfterMidRunRenewal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 60
    Assert-True ([int]$claimsAfterMidRunRenewal.claim_count -ge $Count) 'target claims were not live after mid-run renewal'

    $evalResults = [System.Collections.Generic.List[object]]::new()
    foreach ($lane in @($lanes | Where-Object { (($_.index - 1) % 10) -eq 0 })) {
        $evalMarker = "$($lane.marker)-eval"
        $literal = $evalMarker | ConvertTo-Json -Compress
        $eval = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'browser_evaluate' -Arguments @{ expression = "(() => { document.body.dataset.eval = $literal; return { marker: document.getElementById('marker').textContent, eval: document.body.dataset.eval, value: document.getElementById('value').value, status: document.getElementById('status').textContent }; })()"; return_by_value = $true } -TimeoutSec 45
        $json = Get-JsonText $eval
        Assert-True ($json.Contains($evalMarker)) "lane $($lane.index) evaluate did not return its marker"
        $evalResults.Add([pscustomobject]@{ lane = $lane.index; marker = $evalMarker; result = $eval })
    }

    $navigateResults = [System.Collections.Generic.List[object]]::new()
    foreach ($lane in @($lanes | Where-Object { (($_.index - 1) % 15) -eq 0 })) {
        $navMarker = "$($lane.marker)-navigate"
        $url = "http://127.0.0.1:7700/__issue1220_nav_$($lane.index).html?marker=$([System.Uri]::EscapeDataString($navMarker))"
        $nav = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'cdp_navigate_tab' -Arguments @{ action = 'navigate'; url = $url; wait_timeout_ms = 5000 } -TimeoutSec 45
        $literal = $navMarker | ConvertTo-Json -Compress
        $navEval = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'browser_evaluate' -Arguments @{ expression = "(() => { document.title = $literal; document.body.innerHTML = '<h1 id=`"marker`">' + $literal + '</h1><p id=`"status`">' + $literal + '</p>'; return { marker: document.getElementById('marker').textContent, url: location.href }; })()"; return_by_value = $true } -TimeoutSec 45
        Assert-True ((Get-JsonText $navEval).Contains($navMarker)) "lane $($lane.index) navigate evaluate did not return marker"
        $lane.navigate_marker = $navMarker
        $lane.expected_marker = $navMarker
        $navigateResults.Add([pscustomobject]@{ lane = $lane.index; marker = $navMarker; result = $nav; evaluate = $navEval })
    }

    $screenshotResults = [System.Collections.Generic.List[object]]::new()
    foreach ($lane in @($lanes | Where-Object { (($_.index - 1) % 17) -eq 0 })) {
        $shotPath = Join-Path $artifactRoot ('lane-{0:d2}.png' -f $lane.index)
        $shot = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_act' -Arguments @{ verb = 'screenshot'; path = $shotPath; wait_timeout_ms = 10000 } -TimeoutSec 60
        Assert-RoutineActionOk -Value $shot -Label "screenshot lane $($lane.index)"
        $shotItem = Get-Item -LiteralPath $shotPath -ErrorAction Stop
        Assert-True ($shotItem.Length -gt 1000) "screenshot lane $($lane.index) was too small"
        $screenshotResults.Add([pscustomobject]@{ lane = $lane.index; path = $shotPath; bytes = $shotItem.Length; required_foreground = Get-RequiredForeground $shot })
    }

    $shellResults = [System.Collections.Generic.List[object]]::new()
    foreach ($lane in @($lanes | Where-Object { (($_.index - 1) % 13) -eq 0 })) {
        $shellPath = Join-Path $artifactRoot ('lane-{0:d2}.txt' -f $lane.index)
        $shellMarker = "$($lane.marker)-shell"
        $cmd = "Set-Content -LiteralPath $(Quote-PsSingle $shellPath) -Value $(Quote-PsSingle $shellMarker) -Encoding UTF8"
        $shell = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_act' -Arguments @{ verb = 'run_shell'; command = 'powershell.exe'; args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', $cmd); working_dir = (Get-Location).Path; timeout_ms = 10000 } -TimeoutSec 60
        Assert-RoutineActionOk -Value $shell -Label "run_shell lane $($lane.index)"
        $content = (Get-Content -LiteralPath $shellPath -Raw).Trim()
        Assert-True ($content -eq $shellMarker) "run_shell lane $($lane.index) file readback mismatch"
        $shellResults.Add([pscustomobject]@{ lane = $lane.index; path = $shellPath; marker = $shellMarker; required_foreground = Get-RequiredForeground $shell })
    }

    $normalFocus = Invoke-McpToolMaybe -Bind $Bind -Token $token -SessionId $lanes[3].session_id -NextId ([ref]$lanes[3].next_id) -Name 'target_act' -Arguments @{ verb = 'focus_window'; wait_timeout_ms = 3000 } -TimeoutSec 30
    $normalFocusDenied = (-not $normalFocus.ok) -or ($normalFocus.value -and (Get-PropValue $normalFocus.value 'ok') -eq $false)
    Assert-True ([bool]$normalFocusDenied) 'normal focus_window without lease was not denied'

    $leaseAcquire = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[4].session_id -NextId ([ref]$lanes[4].next_id) -Name 'control_lease_acquire' -Arguments @{ ttl_ms = 10000 } -TimeoutSec 30
    Assert-True ([bool]$leaseAcquire.held -or [bool]$leaseAcquire.is_owner -or [string]$leaseAcquire.outcome -eq 'acquired') 'break-glass lease was not acquired'
    $breakGlassProfile = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[4].session_id -NextId ([ref]$lanes[4].next_id) -Name 'tool_profile_set' -Arguments @{ profile = 'break_glass'; confirm_break_glass = $true; reason = 'issue1220 explicit real foreground lease edge' } -TimeoutSec 30
    $breakGlassFocus = Invoke-McpToolMaybe -Bind $Bind -Token $token -SessionId $lanes[4].session_id -NextId ([ref]$lanes[4].next_id) -Name 'target_act' -Arguments @{ verb = 'focus_window'; wait_timeout_ms = 15000 } -TimeoutSec 60
    $breakGlassFocusOk = $breakGlassFocus.ok -and ((Get-PropValue $breakGlassFocus.value 'ok') -ne $false)
    $breakGlassFocusStatus = if ($breakGlassFocus.ok) { [string](Get-PropValue $breakGlassFocus.value 'status') } else { 'tool_error' }
    $leaseAfterFocus = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[4].session_id -NextId ([ref]$lanes[4].next_id) -Name 'control_lease_status' -Arguments @{} -TimeoutSec 30
    Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[4].session_id -NextId ([ref]$lanes[4].next_id) -Name 'control_lease_release' -Arguments @{} -TimeoutSec 30 | Out-Null
    Invoke-McpTool -Bind $Bind -Token $token -SessionId $lanes[4].session_id -NextId ([ref]$lanes[4].next_id) -Name 'tool_profile_set' -Arguments @{ profile = 'normal_agent'; reason = 'issue1220 restore normal profile after break-glass edge' } -TimeoutSec 30 | Out-Null
    Set-DriverForegroundWindow -Hwnd ([int64]$code.MainWindowHandle)

    Assert-True ($driverSamples.Count -ge 12) 'foreground driver did not produce enough samples'
    $sampleForegrounds = @($driverSamples | Select-Object -ExpandProperty foreground_hwnd -Unique)
    $sampleCursorPairs = @($driverSamples | ForEach-Object { "$($_.cursor_x),$($_.cursor_y)" } | Select-Object -Unique)
    Assert-True ($sampleForegrounds.Count -ge 2) 'foreground driver did not switch foreground windows'
    Assert-True ($sampleCursorPairs.Count -ge 10) 'foreground driver did not move cursor enough'

    $allMarkers = @($lanes | ForEach-Object { [string]$_.expected_marker })
    $readResults = [System.Collections.Generic.List[object]]::new()
    foreach ($lane in $lanes) {
        $read = Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_act' -Arguments @{ verb = 'read'; wait_timeout_ms = 5000 } -TimeoutSec 45
        Assert-RoutineActionOk -Value $read -Label "read lane $($lane.index)"
        $others = @($allMarkers | Where-Object { $_ -ne $lane.expected_marker } | Select-Object -First 8)
        Assert-JsonContainsOnlyOwnSample -Value $read -OwnMarker $lane.expected_marker -OtherMarkers $others -Label "read lane $($lane.index)"
        $readResults.Add([pscustomobject]@{ lane = $lane.index; marker = $lane.expected_marker; required_foreground = Get-RequiredForeground $read })
    }

    foreach ($lane in $lanes) {
        Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'target_claim' -Arguments @{ ttl_ms = 600000 } -TimeoutSec 30 | Out-Null
    }

    $sessionListDuring = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_list' -Arguments @{ view = 'full'; live_only = $true; include_attached_agent_rows = $true; limit = ($Count + 10) } -TimeoutSec 60
    $claimsDuring = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 60
    $leaseDuring = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'control_lease_status' -Arguments @{} -TimeoutSec 30
    Assert-True ([int]$sessionListDuring.foreground_lane_capacity.active_foreground_lane_count -ge $Count) 'session_list dropped lanes before disconnect edge'
    Assert-True ([int]$claimsDuring.claim_count -ge $Count) 'target claims dropped before disconnect edge'
    Assert-True (-not [bool]$leaseDuring.held) 'lease remained held after break-glass release'

    $disconnectLane = $lanes[$lanes.Count - 1]
    Close-McpSession -Bind $Bind -Token $token -SessionId $disconnectLane.session_id
    $disconnectLane.disconnected = $true
    Start-Sleep -Milliseconds 1000
    $sessionListAfterDisconnect = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_list' -Arguments @{ view = 'full'; live_only = $true; include_attached_agent_rows = $true; limit = ($Count + 10) } -TimeoutSec 60
    $claimsAfterDisconnect = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 60
    Assert-True ([int]$claimsAfterDisconnect.claim_count -le ($Count - 1)) 'disconnected session did not release claim'

    $cleanupRows = [System.Collections.Generic.List[object]]::new()
    foreach ($lane in $lanes) {
        if ([bool]$lane.disconnected) { continue }
        $close = Invoke-McpToolMaybe -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'cdp_close_tab' -Arguments @{ cdp_target_id = $lane.target_id } -TimeoutSec 30
        $end = Invoke-McpToolMaybe -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'session_end' -Arguments @{} -TimeoutSec 60
        Close-McpSession -Bind $Bind -Token $token -SessionId $lane.session_id
        $cleanupRows.Add([pscustomobject]@{ lane = $lane.index; close_ok = $close.ok; end_ok = $end.ok; close_error = $close.error; end_error = $end.error })
    }
    Start-Sleep -Milliseconds 1000

    $sessionListFinal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_list' -Arguments @{ view = 'full'; live_only = $true; include_attached_agent_rows = $true; limit = 20 } -TimeoutSec 60
    $claimsFinal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'target_claim_status' -Arguments @{} -TimeoutSec 60
    $leaseFinal = Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'control_lease_status' -Arguments @{} -TimeoutSec 30
    Assert-True ([int]$sessionListFinal.foreground_lane_capacity.claimed_target_lane_count -eq 0) 'claimed lanes remained after cleanup'
    Assert-True ([int]$sessionListFinal.foreground_lane_capacity.active_foreground_lane_count -eq 0) 'active lanes remained after cleanup'
    Assert-True ([int]$claimsFinal.claim_count -eq 0) 'claims remained after cleanup'
    Assert-True (-not [bool]$leaseFinal.held) 'lease remained after cleanup'

    $summary = [ordered]@{
        marker = $runMarker
        count = $Count
        batch_size = $BatchSize
        artifact_root = $artifactRoot
        health = @{
            ok = [bool]$health.ok
            pid = [int]$health.pid
            tool_count = [int]$health.tool_count
            chrome_bridge = [string]$health.subsystems.chrome_bridge.status
        }
        chrome = @{
            hwnd = $chromeHwnd
            title = [string]$chrome.window_title
        }
        sessions = @{
            observer = $observer
            opened_count = $lanes.Count
            first_five = @($lanes | Select-Object -First 5 index, session_id, target_id, marker)
            disconnected = @{ index = $disconnectLane.index; session_id = $disconnectLane.session_id; target_id = $disconnectLane.target_id }
        }
        capacity = @{
            active_lanes = [int]$sessionListAtCapacity.foreground_lane_capacity.active_foreground_lane_count
            claimed_lanes = [int]$sessionListAtCapacity.foreground_lane_capacity.claimed_target_lane_count
            claim_count = [int]$claimsAtCapacity.claim_count
            claim_count_after_midrun_renewal = [int]$claimsAfterMidRunRenewal.claim_count
            capacity_exhausted = [bool]$sessionListAtCapacity.foreground_lane_capacity.capacity_exhausted
            lease_held = [bool]$leaseAtCapacity.held
        }
        human_foreground = @{
            baseline = $baselineWin32
            sample_count = $driverSamples.Count
            distinct_foreground_hwnds = @($sampleForegrounds)
            distinct_cursor_positions = $sampleCursorPairs.Count
            first_samples = @($driverSamples | Select-Object -First 8)
        }
        action_counts = @{
            set_field = $setResults.Count
            press = $pressResults.Count
            read = $readResults.Count
            evaluate = $evalResults.Count
            navigate = $navigateResults.Count
            screenshot = $screenshotResults.Count
            run_shell = $shellResults.Count
        }
        edges = @{
            cross_target_denied = [bool]$crossDenied
            cross_target_status = $crossStatus
            profile_refresh_preserved_capability = [bool]$profileRefresh.snapshot.foreground_capability.profile_preserves_capability
            normal_focus_denied = [bool]$normalFocusDenied
            break_glass_lease_held_after_focus = [bool]$leaseAfterFocus.held
            break_glass_focus_ok = [bool]$breakGlassFocusOk
            break_glass_focus_status = $breakGlassFocusStatus
            break_glass_focus_error = $breakGlassFocus.error
            disconnect_claim_count_after = [int]$claimsAfterDisconnect.claim_count
            disconnect_active_lanes_after = [int]$sessionListAfterDisconnect.foreground_lane_capacity.active_foreground_lane_count
        }
        artifacts = @{
            screenshots = @($screenshotResults | Select-Object lane, path, bytes, required_foreground)
            shell_files = @($shellResults | Select-Object lane, path, marker, required_foreground)
        }
        cleanup = @{
            cleaned_sessions = $cleanupRows.Count
            cleanup_failures = @($cleanupRows | Where-Object { -not $_.close_ok -or -not $_.end_ok }).Count
            final_claim_count = [int]$claimsFinal.claim_count
            final_claimed_lanes = [int]$sessionListFinal.foreground_lane_capacity.claimed_target_lane_count
            final_active_lanes = [int]$sessionListFinal.foreground_lane_capacity.active_foreground_lane_count
            final_lease_held = [bool]$leaseFinal.held
        }
    }
    $summary | ConvertTo-Json -Depth 100
} finally {
    foreach ($lane in @($lanes)) {
        if ($null -eq $lane -or [bool]$lane.disconnected) { continue }
        try { Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'cdp_close_tab' -Arguments @{ cdp_target_id = $lane.target_id } -TimeoutSec 10 | Out-Null } catch {}
        try { Invoke-McpTool -Bind $Bind -Token $token -SessionId $lane.session_id -NextId ([ref]$lane.next_id) -Name 'session_end' -Arguments @{} -TimeoutSec 20 | Out-Null } catch {}
        try { Close-McpSession -Bind $Bind -Token $token -SessionId $lane.session_id } catch {}
    }
    if ($null -ne $observer) {
        try { Invoke-McpTool -Bind $Bind -Token $token -SessionId $observer -NextId ([ref]$nextO) -Name 'session_end' -Arguments @{} -TimeoutSec 20 | Out-Null } catch {}
        try { Close-McpSession -Bind $Bind -Token $token -SessionId $observer } catch {}
    }
    if ($null -ne $startedNotepad -and -not $startedNotepad.HasExited) {
        try { $startedNotepad.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 300 } catch {}
        if (-not $startedNotepad.HasExited) { try { $startedNotepad.Kill() } catch {} }
    }
}
