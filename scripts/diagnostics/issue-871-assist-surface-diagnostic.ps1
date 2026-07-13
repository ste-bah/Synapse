# Supporting diagnostic only. Its output is supporting diagnostic evidence only.
# This script does not perform or accept Full State Verification (FSV). Under
# AGENTS.md D1, an agent must perform FSV manually
# through the strict production MCP client and independently read each physical
# Source of Truth before and after the trigger.
# `synapse-fsv-toast-history` is a public compatibility identity and is
# intentionally unchanged.
param(
    [string]$Bind = '127.0.0.1:7700',
    [string]$TokenPath = "$env:APPDATA\synapse\token.txt",
    [string]$ToastHelperExe = (Join-Path $PSScriptRoot '..\..\target\debug\synapse-fsv-toast-history.exe'),
    [string]$OverlayExe = (Join-Path $PSScriptRoot '..\..\target\debug\synapse-overlay.exe'),
    [string]$SynapseMcpExe = "$env:USERPROFILE\.cargo\bin\synapse-mcp.exe",
    [int]$StartupTimeoutSeconds = 10
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Die($Message) { throw "[issue871-diagnostic] $Message" }

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) { Die $Message }
}

function ConvertTo-JsonCompact {
    param([Parameter(Mandatory = $true)]$Value)
    $Value | ConvertTo-Json -Depth 100 -Compress
}

function Get-WebResponseUtf8Content {
    param([Parameter(Mandatory = $true)]$Response)

    $streamProperty = $Response.PSObject.Properties['RawContentStream']
    if ($streamProperty -and $null -ne $streamProperty.Value) {
        $stream = $streamProperty.Value
        if ($stream.CanSeek) { $stream.Position = 0 }
        $encoding = [System.Text.UTF8Encoding]::new($false, $true)
        $reader = [System.IO.StreamReader]::new($stream, $encoding, $true, 4096, $true)
        try {
            return $reader.ReadToEnd()
        } finally {
            $reader.Dispose()
            if ($stream.CanSeek) { $stream.Position = 0 }
        }
    }
    if ($Response.Content -is [byte[]]) {
        $encoding = [System.Text.UTF8Encoding]::new($false, $true)
        return $encoding.GetString($Response.Content)
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
                if ($line.StartsWith('data:')) {
                    $dataLines += $line.Substring(5).TrimStart()
                }
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
    if (-not [string]::IsNullOrWhiteSpace($SessionId)) {
        $headers['Mcp-Session-Id'] = $SessionId
    }
    $request = [ordered]@{
        jsonrpc = '2.0'
        method = $Method
        params = $Params
    }
    if ($Id -ne 0) { $request['id'] = $Id }
    $body = $request | ConvertTo-Json -Depth 100 -Compress

    $response = Invoke-WebRequest `
        -Uri "http://$Bind/mcp" `
        -Method Post `
        -Headers $headers `
        -ContentType 'application/json' `
        -Body $body `
        -TimeoutSec $TimeoutSec `
        -UseBasicParsing `
        -ErrorAction Stop
    [pscustomobject]@{
        Content = Get-WebResponseUtf8Content -Response $response
        Headers = $response.Headers
        StatusCode = $response.StatusCode
    }
}

function Open-McpSession {
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $initParams = [ordered]@{
        protocolVersion = '2025-06-18'
        capabilities = @{}
        clientInfo = [ordered]@{ name = $Name; version = '1' }
    }
    $initResponse = Invoke-McpHttpPost -Bind $Bind -Token $Token -Method 'initialize' -Params $initParams -Id 1
    $sessionId = @($initResponse.Headers['Mcp-Session-Id'])[0]
    if ([string]::IsNullOrWhiteSpace($sessionId)) {
        Die "initialize did not return Mcp-Session-Id for $Name"
    }
    $initMessage = Read-McpSseJsonResponse -Content $initResponse.Content -Operation 'initialize' -ExpectedId 1
    Assert-True ($null -ne $initMessage.result.capabilities) "initialize response missing capabilities for $Name"
    Invoke-McpHttpPost -Bind $Bind -Token $Token -SessionId $sessionId -Method 'notifications/initialized' -Params @{} | Out-Null
    $sessionId
}

function Invoke-McpMethod {
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][ref]$NextId,
        [Parameter(Mandatory = $true)][string]$Method,
        [Parameter(Mandatory = $true)]$Params,
        [int]$TimeoutSec = 30
    )

    Add-Type -AssemblyName System.Net.Http
    $requestId = $NextId.Value
    $NextId.Value = $NextId.Value + 1
    $response = Invoke-McpHttpPost -Bind $Bind -Token $Token -SessionId $SessionId -Method $Method -Params $Params -Id $requestId -TimeoutSec $TimeoutSec
    $message = Read-McpSseJsonResponse -Content $response.Content -Operation $Method -ExpectedId $requestId
    $message.result
}

function Convert-McpToolResult {
    param(
        [Parameter(Mandatory = $true)]$Message,
        [Parameter(Mandatory = $true)][string]$ToolName
    )

    $isErrorProperty = $Message.result.PSObject.Properties['isError']
    if ($isErrorProperty -and $isErrorProperty.Value -eq $true) {
        $errorText = @($Message.result.content | Where-Object { [string]$_.type -eq 'text' } | Select-Object -First 1).text
        Die "TOOL_CALL_ERROR tool=$ToolName error=$errorText"
    }
    $structured = $Message.result.PSObject.Properties['structuredContent']
    if ($structured -and $null -ne $structured.Value) {
        return $structured.Value
    }
    $text = @($Message.result.content | Where-Object { [string]$_.type -eq 'text' } | Select-Object -First 1).text
    if ([string]::IsNullOrWhiteSpace($text)) {
        return $null
    }
    $text | ConvertFrom-Json
}

function Invoke-McpTool {
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][ref]$NextId,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]$Arguments,
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
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][ref]$NextId,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]$Arguments,
        [int]$TimeoutSec = 120
    )

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
    [pscustomobject]@{
        Client = $client
        Request = $message
        Task = $client.SendAsync($message)
        RequestId = $requestId
        Name = $Name
    }
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
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$SessionId
    )

    $headers = @{
        Authorization = "Bearer $Token"
        Accept = 'application/json, text/event-stream'
        'Mcp-Session-Id' = $SessionId
    }
    try {
        Invoke-WebRequest -Uri "http://$Bind/mcp" -Method Delete -Headers $headers -TimeoutSec 5 -UseBasicParsing -ErrorAction Stop | Out-Null
    } catch {
        Write-Warning "session delete failed for ${SessionId}: $($_.Exception.Message)"
    }
}

function Invoke-DashboardGet {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Token
    )
    Invoke-RestMethod -Uri "http://$Bind$Path" -Headers @{ Authorization = "Bearer $Token" } -TimeoutSec 15
}

function Invoke-DashboardPost {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Body,
        [Parameter(Mandatory = $true)][string]$Token
    )
    $json = $Body | ConvertTo-Json -Depth 80 -Compress
    Invoke-RestMethod -Uri "http://$Bind$Path" -Method Post -Headers @{ Authorization = "Bearer $Token" } -ContentType 'application/json' -Body $json -TimeoutSec 20
}

function Assert-PanelOk {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string[]]$PanelNames
    )
    foreach ($name in $PanelNames) {
        $panel = $State.PSObject.Properties[$name].Value
        Assert-True ($null -ne $panel) "dashboard panel missing: $name"
        $errorText = if ($panel.PSObject.Properties['error']) { [string]$panel.error } else { '' }
        Assert-True ([string]$panel.status -ne 'error') "dashboard panel $name errored: $errorText"
    }
}

function Find-ApprovalItem {
    param(
        [Parameter(Mandatory = $true)]$List,
        [string]$ApprovalId,
        [string]$DedupeKey
    )
    foreach ($entry in @($List.items)) {
        if (-not [string]::IsNullOrWhiteSpace($ApprovalId) -and [string]$entry.item.approval_id -eq $ApprovalId) {
            return $entry
        }
        if (-not [string]::IsNullOrWhiteSpace($DedupeKey) -and [string]$entry.item.dedupe_key -eq $DedupeKey) {
            return $entry
        }
    }
    $null
}

function Wait-ApprovalItem {
    param(
        [Parameter(Mandatory = $true)][string]$ApprovalId,
        [Parameter(Mandatory = $true)][string]$ExpectedStatus,
        [int]$TimeoutSeconds = 20
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $list = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'approval_list' -Arguments @{
            include_terminal = $true
            limit = 200
        }
        $entry = Find-ApprovalItem -List $list -ApprovalId $ApprovalId
        if ($null -ne $entry -and [string]$entry.item.status -eq $ExpectedStatus) {
            return $entry
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    Die "approval $ApprovalId did not reach status $ExpectedStatus"
}

function Wait-PendingDedupe {
    param(
        [Parameter(Mandatory = $true)][string]$DedupeKey,
        [int]$TimeoutSeconds = 20
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $list = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'approval_list' -Arguments @{
            statuses = @('pending')
            include_terminal = $false
            limit = 200
        }
        $entry = Find-ApprovalItem -List $list -DedupeKey $DedupeKey
        if ($null -ne $entry) {
            return $entry
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    Die "pending approval with dedupe_key=$DedupeKey did not appear"
}

function Get-DashboardApprovalRow {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$ApprovalId
    )
    foreach ($row in @($State.approvals.data.rows)) {
        if ([string]$row.item.approval_id -eq $ApprovalId) { return $row }
    }
    $null
}

function Count-PendingRows {
    param([Parameter(Mandatory = $true)]$Rows)
    @($Rows | Where-Object { [string]$_.item.status -eq 'pending' }).Count
}

function Invoke-ToastHelper {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    $output = & $ToastHelperExe @Arguments
    if ($LASTEXITCODE -ne 0) {
        Die "toast helper failed args=$($Arguments -join ' ')"
    }
    ($output -join "`n") | ConvertFrom-Json
}

function Invoke-OverlayJson {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    $oldToken = $env:SYNAPSE_BEARER_TOKEN
    $oldBase = $env:SYNAPSE_TRAY_BASE_URL
    try {
        $env:SYNAPSE_BEARER_TOKEN = $token
        $env:SYNAPSE_TRAY_BASE_URL = "http://$Bind"
        $output = & $OverlayExe @Arguments
        if ($LASTEXITCODE -ne 0) {
            Die "synapse-overlay failed args=$($Arguments -join ' ')"
        }
        ($output -join "`n") | ConvertFrom-Json
    } finally {
        $env:SYNAPSE_BEARER_TOKEN = $oldToken
        $env:SYNAPSE_TRAY_BASE_URL = $oldBase
    }
}

function Invoke-ApprovalProtocolUri {
    param([Parameter(Mandatory = $true)][string]$Uri)
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $Uri
    $psi.UseShellExecute = $true
    $psi.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $process = [System.Diagnostics.Process]::Start($psi)
    if ($null -ne $process) {
        $process.WaitForExit(10000) | Out-Null
    }
}

function Redact-ActivationUri {
    param([string]$Uri)
    if ([string]::IsNullOrWhiteSpace($Uri)) { return $Uri }
    $Uri -replace 'token=[^&]+', 'token=<redacted>'
}

function Ensure-Binary {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$CargoArgs,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if (Test-Path -LiteralPath $Path -PathType Leaf) { return }
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) {
        Die "cargo build failed for $Name"
    }
    Assert-True (Test-Path -LiteralPath $Path -PathType Leaf) "$Name binary not found after build: $Path"
}

function Restore-RecorderState {
    param([bool]$DesiredPaused)
    try {
        $state = Invoke-DashboardGet -Path '/dashboard/tray-state.json' -Token $token
        $currentPaused = [bool]$state.timeline.data.recorder.paused
        if ($currentPaused -eq $DesiredPaused) { return }
        if ($DesiredPaused) {
            Invoke-DashboardPost -Path '/dashboard/timeline/pause' -Body @{} -Token $token | Out-Null
        } else {
            Invoke-DashboardPost -Path '/dashboard/timeline/resume' -Body @{} -Token $token | Out-Null
        }
    } catch {
        Write-Warning "failed to restore recorder state: $($_.Exception.Message)"
    }
}

if (-not (Test-Path -LiteralPath $TokenPath -PathType Leaf)) {
    Die "token file not found: $TokenPath"
}
$token = (Get-Content -Raw -LiteralPath $TokenPath).Trim()
Assert-True ($token.Length -ge 16) "token too short at $TokenPath"
if (-not (Test-Path -LiteralPath $SynapseMcpExe -PathType Leaf)) {
    $fallback = Join-Path $PSScriptRoot '..\..\target\release\synapse-mcp.exe'
    if (Test-Path -LiteralPath $fallback -PathType Leaf) {
        $SynapseMcpExe = $fallback
    } else {
        Die "synapse-mcp.exe not found at $SynapseMcpExe or $fallback"
    }
}

Ensure-Binary -Path $ToastHelperExe -CargoArgs @('build', '-p', 'synapse-mcp', '--bin', 'synapse-fsv-toast-history') -Name 'synapse-fsv-toast-history'
Ensure-Binary -Path $OverlayExe -CargoArgs @('build', '-p', 'synapse-overlay') -Name 'synapse-overlay'

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$marker = "issue871-diagnostic-$stamp"
$screenshotPath = Join-Path ([System.IO.Path]::GetTempPath()) "$marker-dashboard.png"
$sessionA = $null
$sessionB = $null
$nextA = 2
$nextB = 2
$dashboardTabId = $null
$dashboardWindowHwnd = $null
$toastTag = $null
$initialRecorderPaused = $null
$gateApprovalId = $null
$gateCall = $null

try {
    $deadline = (Get-Date).AddSeconds($StartupTimeoutSeconds)
    $healthRest = $null
    do {
        try {
            $healthRest = Invoke-RestMethod -Uri "http://$Bind/health" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 2
        } catch {
            $healthRest = $null
        }
    } while ($null -eq $healthRest -and (Get-Date) -lt $deadline)
    Assert-True ($null -ne $healthRest -and $healthRest.ok -eq $true) "daemon health failed at http://$Bind/health"

    $sessionA = Open-McpSession -Bind $Bind -Token $token -Name 'issue871-diagnostic-agent-a'
    $sessionB = Open-McpSession -Bind $Bind -Token $token -Name 'issue871-diagnostic-agent-b'

    $toolsList = Invoke-McpMethod -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Method 'tools/list' -Params @{}
    $toolNames = @($toolsList.tools | ForEach-Object { [string]$_.name })
    foreach ($requiredTool in @(
        'health', 'tool_profile_status', 'session_list', 'approval_request', 'approval_list',
        'approval_gate', 'timeline_stats', 'timeline_pause', 'timeline_resume',
        'storage_inspect', 'window_list', 'browser_tabs', 'browser_content',
        'browser_evaluate'
    )) {
        Assert-True ($toolNames -contains $requiredTool) "required tool missing from normal profile: $requiredTool"
    }

    $health = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'health' -Arguments @{}
    $profile = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'tool_profile_status' -Arguments @{}
    $profileSnapshot = if ($profile.PSObject.Properties['snapshot']) { $profile.snapshot } else { $profile }
    Assert-True ([int]$health.tool_count -eq @($profileSnapshot.visible_tool_names).Count) "tool profile visible count does not match health"

    $sessions = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'session_list' -Arguments @{
        view = 'full'
        include_closed = $false
        limit = 100
    }
    $liveSessionIds = @($sessions.sessions | Where-Object { [string]$_.lifecycle -eq 'live' } | ForEach-Object { [string]$_.session_id })
    Assert-True ($liveSessionIds -contains $sessionA) "session_list missing live sessionA $sessionA"
    Assert-True ($liveSessionIds -contains $sessionB) "session_list missing live sessionB $sessionB"

    $windows = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'window_list' -Arguments @{
        process_name_contains = 'chrome'
        exclude_minimized = $false
    }
    $chromeWindows = @($windows.windows | Where-Object { $_.is_chromium -eq $true })
    Assert-True (@($chromeWindows).Count -gt 0) 'no existing Chrome window found'
    $dashboardWindow = @($chromeWindows | Where-Object { [string]$_.window_title -like '*Synapse Command Center*' } | Select-Object -First 1)
    if ($null -eq $dashboardWindow) { $dashboardWindow = @($chromeWindows | Select-Object -First 1) }
    $dashboardWindowHwnd = [int64]$dashboardWindow.hwnd

    $dashboardUrl = "http://$Bind/dashboard?diagnostic=$marker#/system"
    $newTab = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'browser_tabs' -Arguments @{
        operation = 'new'
        window_hwnd = $dashboardWindowHwnd
        url = $dashboardUrl
    } -TimeoutSec 30
    if ($newTab.PSObject.Properties['mutation'] -and $newTab.mutation.PSObject.Properties['opened_cdp_target_id']) {
        $dashboardTabId = [string]$newTab.mutation.opened_cdp_target_id
    } elseif ($newTab.PSObject.Properties['cdp_target_id']) {
        $dashboardTabId = [string]$newTab.cdp_target_id
    } else {
        $dashboardTab = @($newTab.tabs | Where-Object {
            $_.PSObject.Properties['url'] -and [string]$_.url -eq $dashboardUrl
        } | Select-Object -First 1)
        if ($null -ne $dashboardTab -and $dashboardTab.PSObject.Properties['cdp_target_id']) {
            $dashboardTabId = [string]$dashboardTab.cdp_target_id
        }
    }
    Assert-True (-not [string]::IsNullOrWhiteSpace($dashboardTabId)) 'browser_tabs new did not return a cdp target id'
    $content = $null
    $contentDeadline = (Get-Date).AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 250
        $content = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'browser_content' -Arguments @{
            cdp_target_id = $dashboardTabId
            window_hwnd = $dashboardWindowHwnd
            max_bytes = 262144
        } -TimeoutSec 30
        if ([string]$content.url -like "http://$Bind/dashboard*" -and [string]$content.title -like '*Synapse*') {
            break
        }
    } while ((Get-Date) -lt $contentDeadline)
    Assert-True ([string]$content.url -like "http://$Bind/dashboard*") "dashboard browser URL mismatch: $($content.url)"
    Assert-True ([string]$content.title -like '*Synapse*') "dashboard browser title mismatch: $($content.title)"
    $domReadback = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'browser_evaluate' -Arguments @{
        cdp_target_id = $dashboardTabId
        window_hwnd = $dashboardWindowHwnd
        expression = "({ title: document.title, url: location.href, readyState: document.readyState, text: document.body.innerText.slice(0, 2000) })"
        return_by_value = $true
        await_promise = $true
    } -TimeoutSec 30
    Assert-True ([string]$domReadback.value.title -like '*Synapse*') "dashboard DOM title mismatch: $($domReadback.value.title)"
    Assert-True ([string]$domReadback.value.text -like '*Synapse*') 'dashboard DOM text missing Synapse'
    $screenshotBytes = 0
    $screenshotError = $null
    if ($toolNames -contains 'browser_screenshot') {
        try {
            Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'browser_screenshot' -Arguments @{
                cdp_target_id = $dashboardTabId
                window_hwnd = $dashboardWindowHwnd
                path = $screenshotPath
                scope = 'viewport'
                format = 'png'
                overwrite = $true
                wait_timeout_ms = 5000
            } -TimeoutSec 45 | Out-Null
            if (Test-Path -LiteralPath $screenshotPath -PathType Leaf) {
                $screenshotInfo = Get-Item -LiteralPath $screenshotPath
                $screenshotBytes = [int64]$screenshotInfo.Length
            }
        } catch {
            $screenshotError = $_.Exception.Message
        }
    }

    $dashboardState = Invoke-DashboardGet -Path '/dashboard/state.json' -Token $token
    $trayState = Invoke-DashboardGet -Path '/dashboard/tray-state.json' -Token $token
    Assert-PanelOk -State $dashboardState -PanelNames @(
        'daemon', 'sessions', 'lease', 'storage', 'timeline', 'events',
        'command_audit', 'tasks', 'approvals', 'suggestions', 'armed_runs',
        'agent_transcripts', 'agent_cost', 'agent_stats', 'context', 'hygiene'
    )
    Assert-True ([int]$dashboardState.daemon.data.pid -eq [int]$healthRest.pid) 'dashboard daemon pid does not match /health'
    Assert-True ([int]$dashboardState.daemon.data.tool_count -eq [int]$healthRest.tool_count) 'dashboard daemon tool_count does not match /health'
    Assert-True ([string]$dashboardState.daemon.data.tool_surface_sha256 -eq [string]$healthRest.tool_surface_sha256) 'dashboard tool surface hash mismatch'
    $timelineStats = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'timeline_stats' -Arguments @{}
    Assert-True ([int64]$dashboardState.timeline.data.total_rows -eq [int64]$timelineStats.total_rows) 'dashboard timeline total_rows mismatch'
    Assert-True ([bool]$dashboardState.timeline.data.recorder.paused -eq [bool]$timelineStats.recorder.paused) 'dashboard timeline paused mismatch'
    $storage = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'storage_inspect' -Arguments @{}
    Assert-True ([int64]$dashboardState.storage.data.cf_row_counts.CF_KV -eq [int64]$storage.cf_row_counts.CF_KV) 'dashboard storage CF_KV row count mismatch'

    $initialRecorderPaused = [bool]$trayState.timeline.data.recorder.paused
    $overlayStatus = Invoke-OverlayJson -Arguments @('--status-once')
    Assert-True ([bool]$overlayStatus.recorder_paused -eq [bool]$trayState.timeline.data.recorder.paused) 'tray status-once recorder mismatch'
    Assert-True ([int]$overlayStatus.daemon_pid -eq [int]$health.pid) 'tray status-once daemon pid mismatch'
    Assert-True ([int]$overlayStatus.pending_approvals -eq (Count-PendingRows -Rows $trayState.approvals.data.rows)) 'tray pending approvals mismatch'

    $toastDedupe = "issue871-toast-$marker"
    $toastTitle = "Issue #871 diagnostic toast $marker"
    $toastBody = "Actionable approval toast for the assist-surface diagnostic."
    $request = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'approval_request' -Arguments @{
        kind = 'suggestion'
        title = $toastTitle
        body = $toastBody
        dedupe_key = $toastDedupe
        payload_json = (ConvertTo-JsonCompact @{ marker = $marker; scenario = 'toast-accept' })
        timeout_ms = 600000
        timeout_decision = 'ignored'
        notify = $true
        suppress_popup = $false
        destructive = $false
    } -TimeoutSec 45
    $toastApprovalId = [string]$request.item.approval_id
    $toastTag = [string]$request.item.toast.notify_tag
    Assert-True ([string]$request.item.status -eq 'pending') 'toast approval was not pending after request'
    Assert-True ([bool]$request.item.toast.requested) 'toast delivery was not requested'
    $toastUnavailableReasonProperty = $request.item.toast.PSObject.Properties['unavailable_reason']
    $toastUnavailableReason = if ($toastUnavailableReasonProperty) { [string]$toastUnavailableReasonProperty.Value } else { '<missing unavailable_reason>' }
    Assert-True ([bool]$request.item.toast.actionable_buttons) "toast was not actionable: $toastUnavailableReason toast=$(ConvertTo-JsonCompact $request.item.toast)"
    Assert-True ([bool]$request.item.toast.verified_in_history) 'toast was not verified in Action Center history'
    Assert-True (-not [string]::IsNullOrWhiteSpace($toastTag)) 'toast notify_tag missing'

    $toastReadback = Invoke-ToastHelper -Arguments @('extract-approval', '--tag', $toastTag)
    Assert-True (@($toastReadback.texts | Where-Object { $_ -eq $toastTitle }).Count -eq 1) 'physical toast title text mismatch'
    Assert-True (@($toastReadback.texts | Where-Object { $_ -eq $toastBody }).Count -eq 1) 'physical toast body text mismatch'
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$toastReadback.accept_uri)) 'physical toast accept URI missing'
    Assert-True ([string]$toastReadback.accept_uri -like "*approval_id=$toastApprovalId*") 'physical toast accept URI approval id mismatch'
    Assert-True ([string]$toastReadback.accept_uri -like "*activation_id=$($request.item.toast.activation_id)*") 'physical toast activation id mismatch'

    $dashboardWithToast = Invoke-DashboardGet -Path '/dashboard/state.json' -Token $token
    $trayWithToast = Invoke-DashboardGet -Path '/dashboard/tray-state.json' -Token $token
    $dashToastRow = Get-DashboardApprovalRow -State $dashboardWithToast -ApprovalId $toastApprovalId
    $trayToastRow = Get-DashboardApprovalRow -State $trayWithToast -ApprovalId $toastApprovalId
    Assert-True ($null -ne $dashToastRow) 'dashboard approvals panel missing pending toast row'
    Assert-True ($null -ne $trayToastRow) 'tray approvals panel missing pending toast row'
    Assert-True ([string]$dashToastRow.item_row.value_sha256 -eq [string]$request.item_row.value_sha256) 'dashboard approval row hash mismatch'
    Assert-True ([string]$trayToastRow.item_row.value_sha256 -eq [string]$request.item_row.value_sha256) 'tray approval row hash mismatch'

    Invoke-ApprovalProtocolUri -Uri ([string]$toastReadback.accept_uri)
    $acceptedToast = Wait-ApprovalItem -ApprovalId $toastApprovalId -ExpectedStatus 'accepted' -TimeoutSeconds 20
    Assert-True ([string]$acceptedToast.item.decided_by_session -eq 'approval_protocol') "toast activation did not flow through approval_protocol: $($acceptedToast.item.decided_by_session)"
    Invoke-ToastHelper -Arguments @('remove', '--tag', $toastTag) | Out-Null

    $gateToolUseId = "issue871-gate-$stamp"
    $gateSpawn = "agent-spawn-issue871-$stamp"
    $gateDedupe = "gate:${gateSpawn}:${gateToolUseId}"
    $gateInput = @{ command = "git push origin issue871-diagnostic-$stamp" }
    $gateCall = Start-McpToolCallAsync -Bind $Bind -Token $token -SessionId $sessionB -NextId ([ref]$nextB) -Name 'approval_gate' -Arguments @{
        tool_name = 'Bash'
        tool_use_id = $gateToolUseId
        spawn_id = $gateSpawn
        input = $gateInput
    } -TimeoutSec 120
    $gatePending = Wait-PendingDedupe -DedupeKey $gateDedupe -TimeoutSeconds 20
    $gateApprovalId = [string]$gatePending.item.approval_id
    $dashboardWithGate = Invoke-DashboardGet -Path '/dashboard/state.json' -Token $token
    Assert-True ($null -ne (Get-DashboardApprovalRow -State $dashboardWithGate -ApprovalId $gateApprovalId)) 'dashboard approvals panel missing blocked gate row'
    $gateDecision = Invoke-DashboardPost -Path '/dashboard/approval/decide' -Body @{
        approval_id = $gateApprovalId
        decision = 'accept'
        note = "issue871 diagnostic dashboard accept resumes blocked approval_gate"
    } -Token $token
    Assert-True ([string]$gateDecision.decision.after_status -eq 'accepted') 'dashboard approval decide did not accept gate row'
    $gateVerdict = Receive-McpToolCallAsync -Call $gateCall
    $gateCall = $null
    Assert-True ([string]$gateVerdict.behavior -eq 'allow') "approval_gate verdict was not allow: $(ConvertTo-JsonCompact $gateVerdict)"
    Assert-True ([string]$gateVerdict.updatedInput.command -eq [string]$gateInput.command) 'approval_gate updatedInput command mismatch'

    $toggle1 = Invoke-OverlayJson -Arguments @('--toggle-once')
    Assert-True ([bool]$toggle1.before.recorder_paused -eq $initialRecorderPaused) 'tray toggle before state did not match initial recorder state'
    Assert-True ([bool]$toggle1.after.recorder_paused -ne [bool]$toggle1.before.recorder_paused) 'tray toggle-once did not change recorder state'
    $toggleState = Invoke-DashboardGet -Path '/dashboard/tray-state.json' -Token $token
    Assert-True ([bool]$toggleState.timeline.data.recorder.paused -eq [bool]$toggle1.after.recorder_paused) 'dashboard did not reflect tray toggle state'
    $toggle2 = Invoke-OverlayJson -Arguments @('--toggle-once')
    Assert-True ([bool]$toggle2.after.recorder_paused -eq $initialRecorderPaused) 'second tray toggle did not restore initial recorder state'

    $finalDashboard = Invoke-DashboardGet -Path '/dashboard/state.json' -Token $token
    $finalTray = Invoke-DashboardGet -Path '/dashboard/tray-state.json' -Token $token
    $finalApprovals = Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'approval_list' -Arguments @{
        statuses = @('pending')
        include_terminal = $false
        limit = 200
    }
    Assert-True ($null -eq (Get-DashboardApprovalRow -State $finalDashboard -ApprovalId $toastApprovalId)) 'dashboard still shows accepted toast row as pending'
    Assert-True ($null -eq (Get-DashboardApprovalRow -State $finalDashboard -ApprovalId $gateApprovalId)) 'dashboard still shows accepted gate row as pending'
    Assert-True ((Count-PendingRows -Rows $finalTray.approvals.data.rows) -eq @($finalApprovals.items).Count) 'final tray pending count does not match approval_list'

    $summary = [ordered]@{
        issue = 871
        marker = $marker
        daemon = [ordered]@{
            pid = [int]$healthRest.pid
            bind = $Bind
            daemon_tool_count = [int]$healthRest.tool_count
            daemon_tool_surface_sha256 = [string]$healthRest.tool_surface_sha256
            visible_tool_count = [int]$health.tool_count
            visible_tool_surface_sha256 = [string]$health.tool_surface_sha256
            chrome_bridge_status = [string]$healthRest.subsystems.chrome_bridge.status
        }
        concurrent_sessions = [ordered]@{
            session_a = $sessionA
            session_b = $sessionB
            live_session_count = @($liveSessionIds).Count
            dashboard_total_count = [int]$dashboardState.sessions.data.total_count
        }
        browser_dashboard = [ordered]@{
            existing_chrome_hwnd = $dashboardWindowHwnd
            cdp_target_id = $dashboardTabId
            url = [string]$content.url
            dom_title = [string]$domReadback.value.title
            screenshot_path = $screenshotPath
            screenshot_bytes = [int64]$screenshotBytes
            screenshot_error = $screenshotError
        }
        E1_toast_accept = [ordered]@{
            approval_id = $toastApprovalId
            toast_tag = $toastTag
            activation_id = [string]$request.item.toast.activation_id
            toast_xml_sha256 = [string]$toastReadback.xml_sha256
            accept_uri_redacted = Redact-ActivationUri -Uri ([string]$toastReadback.accept_uri)
            after_status = [string]$acceptedToast.item.status
            decided_by_session = [string]$acceptedToast.item.decided_by_session
            item_row_sha256 = [string]$acceptedToast.item_row.value_sha256
        }
        E2_queue_sot = [ordered]@{
            dashboard_pending_row_hash = [string]$dashToastRow.item_row.value_sha256
            tray_pending_row_hash = [string]$trayToastRow.item_row.value_sha256
            mcp_pending_row_hash = [string]$request.item_row.value_sha256
        }
        E3_dashboard_sot = [ordered]@{
            daemon_pid_match = $true
            timeline_total_rows = [int64]$timelineStats.total_rows
            storage_cf_kv_rows = [int64]$storage.cf_row_counts.CF_KV
            panel_statuses = ($dashboardState.PSObject.Properties | Where-Object { $_.Value.PSObject.Properties['status'] } | ForEach-Object { "$($_.Name)=$($_.Value.status)" }) -join ';'
        }
        E4_tray_control = [ordered]@{
            status_once_pending_approvals = [int]$overlayStatus.pending_approvals
            initial_recorder_paused = $initialRecorderPaused
            toggled_recorder_paused = [bool]$toggle1.after.recorder_paused
            restored_recorder_paused = [bool]$toggle2.after.recorder_paused
        }
        E5_controls_control = [ordered]@{
            gate_approval_id = $gateApprovalId
            dashboard_decision_status = [string]$gateDecision.decision.after_status
            gate_verdict = [string]$gateVerdict.behavior
            updated_command = [string]$gateVerdict.updatedInput.command
        }
        discrepancies = @()
    }
    $summary | ConvertTo-Json -Depth 100
} finally {
    if ($null -ne $initialRecorderPaused) {
        Restore-RecorderState -DesiredPaused $initialRecorderPaused
    }
    if ($null -ne $gateCall -and -not $gateCall.Task.IsCompleted -and -not [string]::IsNullOrWhiteSpace($gateApprovalId)) {
        try {
            Invoke-DashboardPost -Path '/dashboard/approval/decide' -Body @{
                approval_id = $gateApprovalId
                decision = 'decline'
                note = 'issue871 diagnostic cleanup after interrupted gate run'
            } -Token $token | Out-Null
        } catch {
            Write-Warning "failed to cleanup gate approval ${gateApprovalId}: $($_.Exception.Message)"
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($toastTag) -and (Test-Path -LiteralPath $ToastHelperExe -PathType Leaf)) {
        try { Invoke-ToastHelper -Arguments @('remove', '--tag', $toastTag) | Out-Null } catch {}
    }
    if (-not [string]::IsNullOrWhiteSpace($dashboardTabId) -and -not [string]::IsNullOrWhiteSpace([string]$dashboardWindowHwnd)) {
        try {
            Invoke-McpTool -Bind $Bind -Token $token -SessionId $sessionA -NextId ([ref]$nextA) -Name 'browser_tabs' -Arguments @{
                operation = 'close'
                window_hwnd = $dashboardWindowHwnd
                cdp_target_id = $dashboardTabId
            } -TimeoutSec 20 | Out-Null
        } catch {
            Write-Warning "failed to close dashboard tab ${dashboardTabId}: $($_.Exception.Message)"
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($sessionB)) {
        Close-McpSession -Bind $Bind -Token $token -SessionId $sessionB
    }
    if (-not [string]::IsNullOrWhiteSpace($sessionA)) {
        Close-McpSession -Bind $Bind -Token $token -SessionId $sessionA
    }
}
