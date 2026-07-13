# Supporting diagnostic only. Its output is supporting diagnostic evidence only.
# This script does not perform or accept Full State Verification (FSV). Under
# AGENTS.md D1, an agent must perform FSV manually
# through the strict production MCP client and independently read each physical
# Source of Truth before and after the trigger.
param(
    [string]$SynapseMcpExe = (Join-Path $PSScriptRoot '..\..\target\release\synapse-mcp.exe'),
    [string]$ProfileDir = "$env:USERPROFILE\.cargo\bin\profiles",
    [string]$TokenPath = "$env:APPDATA\synapse\token.txt",
    [int]$StartupTimeoutSeconds = 20,
    [int]$LatencyBudgetMs = 2000
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Die($Message) { throw "[issue857-diagnostic] $Message" }

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) { Die $Message }
}

function Get-FreeLoopbackBind {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse('127.0.0.1'), 0)
    $listener.Start()
    try {
        $port = $listener.LocalEndpoint.Port
    } finally {
        $listener.Stop()
    }
    "127.0.0.1:$port"
}

function Convert-LocalDateTimeToUnixNs {
    param([Parameter(Mandatory = $true)][datetime]$LocalDateTime)

    $unspecified = [datetime]::SpecifyKind($LocalDateTime, [DateTimeKind]::Unspecified)
    $offset = [System.TimeZoneInfo]::Local.GetUtcOffset($unspecified)
    $dto = [DateTimeOffset]::new($unspecified, $offset)
    [int64]($dto.ToUnixTimeMilliseconds() * 1000000)
}

function Convert-DateToUnixNs {
    param([Parameter(Mandatory = $true)][datetime]$LocalDate)
    Convert-LocalDateTimeToUnixNs -LocalDateTime $LocalDate.Date
}

function Get-LastFullWeekMonday {
    $today = (Get-Date).Date
    $daysSinceMonday = (([int]$today.DayOfWeek + 6) % 7)
    $today.AddDays(-$daysSinceMonday - 7)
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

    $message = $null
    $trimmed = $Content.Trim()
    if ($trimmed.StartsWith('{')) {
        $message = $trimmed | ConvertFrom-Json
    } else {
        foreach ($block in ($Content -split "(`r?`n){2,}")) {
            foreach ($line in ($block -split "`r?`n")) {
                if (-not $line.StartsWith('data:')) { continue }
                $data = $line.Substring(5).Trim()
                if ($data.StartsWith('{')) {
                    $message = $data | ConvertFrom-Json
                    break
                }
            }
            if ($null -ne $message) { break }
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
        $errorJson = $errorProperty.Value | ConvertTo-Json -Depth 12 -Compress
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
    $body = $request | ConvertTo-Json -Depth 80 -Compress

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
        [Parameter(Mandatory = $true)][string]$Token
    )

    $initParams = [ordered]@{
        protocolVersion = '2025-06-18'
        capabilities = @{}
        clientInfo = [ordered]@{ name = 'issue857-diagnostic'; version = '1' }
    }
    $initResponse = Invoke-McpHttpPost -Bind $Bind -Token $Token -Method 'initialize' -Params $initParams -Id 1
    $sessionId = @($initResponse.Headers['Mcp-Session-Id'])[0]
    if ([string]::IsNullOrWhiteSpace($sessionId)) {
        Die "initialize did not return Mcp-Session-Id"
    }
    $initMessage = Read-McpSseJsonResponse -Content $initResponse.Content -Operation 'initialize' -ExpectedId 1
    Assert-True ($null -ne $initMessage.result.capabilities) 'initialize response missing capabilities'
    Invoke-McpHttpPost -Bind $Bind -Token $Token -SessionId $sessionId -Method 'notifications/initialized' -Params @{} | Out-Null
    $sessionId
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
    $isErrorProperty = $message.result.PSObject.Properties['isError']
    if ($isErrorProperty -and $isErrorProperty.Value -eq $true) {
        $errorText = @($message.result.content | Where-Object { [string]$_.type -eq 'text' } | Select-Object -First 1).text
        Die "TOOL_CALL_ERROR tool=$Name error=$errorText"
    }
    $structured = $message.result.PSObject.Properties['structuredContent']
    if ($structured -and $null -ne $structured.Value) {
        return $structured.Value
    }
    $text = @($message.result.content | Where-Object { [string]$_.type -eq 'text' } | Select-Object -First 1).text
    if ([string]::IsNullOrWhiteSpace($text)) {
        return $null
    }
    $text | ConvertFrom-Json
}

function Invoke-McpToolTimed {
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][ref]$NextId,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]$Arguments,
        [int]$BudgetMs = 2000,
        [int]$TimeoutSec = 60
    )

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $result = Invoke-McpTool -Bind $Bind -Token $Token -SessionId $SessionId -NextId $NextId -Name $Name -Arguments $Arguments -TimeoutSec $TimeoutSec
    $sw.Stop()
    Assert-True ($sw.ElapsedMilliseconds -le $BudgetMs) "$Name latency $($sw.ElapsedMilliseconds)ms exceeded budget ${BudgetMs}ms"
    [pscustomobject]@{
        result = $result
        elapsed_ms = [int64]$sw.ElapsedMilliseconds
        budget_ms = $BudgetMs
    }
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

    $requestId = $NextId.Value
    $NextId.Value = $NextId.Value + 1
    $response = Invoke-McpHttpPost -Bind $Bind -Token $Token -SessionId $SessionId -Method $Method -Params $Params -Id $requestId -TimeoutSec $TimeoutSec
    $message = Read-McpSseJsonResponse -Content $response.Content -Operation $Method -ExpectedId $requestId
    $message.result
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
        Write-Warning "session delete failed: $($_.Exception.Message)"
    }
}

function Seed-TimelineRow {
    param(
        [Parameter(Mandatory = $true)][string]$Bind,
        [Parameter(Mandatory = $true)][string]$Token,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][ref]$NextId,
        [Parameter(Mandatory = $true)][string]$Prefix,
        [Parameter(Mandatory = $true)][Int64]$TsNs,
        [Parameter(Mandatory = $true)]$ValueJson
    )

    $put = Invoke-McpTool -Bind $Bind -Token $Token -SessionId $SessionId -NextId $NextId -Name 'storage_put_probe_rows' -Arguments @{
        cf_name = 'CF_TIMELINE'
        key_prefix = $Prefix
        rows = 1
        value_bytes = 0
        value_json = $ValueJson
        ts_ns_start = $TsNs
        key_mode = 'timeline_ts'
    }
    Assert-True ([int]$put.rows_added -eq 1) "seed failed prefix=$Prefix rows_added=$($put.rows_added)"
}

function New-FocusRow {
    param(
        [Parameter(Mandatory = $true)][string]$Marker,
        [Parameter(Mandatory = $true)][string]$App,
        [Parameter(Mandatory = $true)][string]$Title
    )
    @{
        record_version = 1
        kind = 'focus_change'
        actor = @{ actor = 'human' }
        app = $App
        payload = @{
            title = $Title
            pid = 857
            hwnd = 857
            source = 'issue857-diagnostic'
            diagnostic_marker = $Marker
        }
    }
}

function New-IdleRow {
    param([Parameter(Mandatory = $true)][string]$Marker)
    @{
        record_version = 1
        kind = 'idle_start'
        actor = @{ actor = 'human' }
        payload = @{
            idle_ms_at_detection = 180000
            idle_timeout_ms = 180000
            source = 'issue857-diagnostic'
            diagnostic_marker = $Marker
        }
    }
}

function Transition-Kinds {
    param($Outcome)
    @($Outcome.transitions | ForEach-Object { [string]$_.kind })
}

if (-not (Test-Path -LiteralPath $SynapseMcpExe -PathType Leaf)) {
    $fallback = "$env:USERPROFILE\.cargo\bin\synapse-mcp.exe"
    if (Test-Path -LiteralPath $fallback -PathType Leaf) {
        $SynapseMcpExe = $fallback
    } else {
        Die "synapse-mcp.exe not found at $SynapseMcpExe or $fallback"
    }
}
if (-not (Test-Path -LiteralPath $ProfileDir -PathType Container)) {
    Die "profile dir not found: $ProfileDir"
}
if (-not (Test-Path -LiteralPath $TokenPath -PathType Leaf)) {
    Die "token file not found: $TokenPath"
}
$token = (Get-Content -Raw -LiteralPath $TokenPath).Trim()
Assert-True ($token.Length -ge 16) "token too short at $TokenPath"
$env:SYNAPSE_BEARER_TOKEN = $token

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$marker = "issue857-diagnostic-$stamp"
$bind = Get-FreeLoopbackBind
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "synapse-issue857-diagnostic-$stamp"
$dbPath = Join-Path $tempRoot 'db'
$logPath = Join-Path $tempRoot 'daemon.log'
$errPath = Join-Path $tempRoot 'daemon.err.log'
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null

$process = $null
$sessionId = $null
$nextId = 2
try {
    $args = @(
        '--mode', 'http',
        '--bind', $bind,
        '--db', $dbPath,
        '--profile-dir', $ProfileDir,
        '--log-level', 'info'
    )
    $process = Start-Process `
        -FilePath $SynapseMcpExe `
        -ArgumentList $args `
        -WindowStyle Hidden `
        -PassThru `
        -RedirectStandardOutput $logPath `
        -RedirectStandardError $errPath

    $deadline = (Get-Date).AddSeconds($StartupTimeoutSeconds)
    $health = $null
    do {
        Start-Sleep -Milliseconds 250
        try {
            $health = Invoke-RestMethod -Uri "http://$bind/health" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 2
        } catch {
            $health = $null
        }
    } while ($null -eq $health -and (Get-Date) -lt $deadline)
    if ($null -eq $health) {
        Die "temporary daemon did not become healthy bind=$bind stdout=$logPath stderr=$errPath"
    }

    $sessionId = Open-McpSession -Bind $bind -Token $token
    $toolsList = Invoke-McpMethod -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Method 'tools/list' -Params @{}
    $toolNames = @($toolsList.tools | ForEach-Object { [string]$_.name })
    foreach ($requiredTool in @(
        'storage_put_probe_rows', 'timeline_search', 'episode_segment', 'episode_list',
        'episode_get', 'routine_mine', 'routine_list', 'routine_inspect',
        'routine_update', 'intent_current', 'intent_detect_tick', 'routine_feedback',
        'subscribe', 'subscribe_cancel', 'storage_inspect', 'session_end'
    )) {
        Assert-True ($toolNames -contains $requiredTool) "required tool missing from normal profile: $requiredTool"
    }

    $weekMonday = Get-LastFullWeekMonday
    $liveDay = $weekMonday.AddDays(7)
    $trainingStartNs = Convert-DateToUnixNs -LocalDate $weekMonday
    $trainingEndNs = Convert-DateToUnixNs -LocalDate $liveDay
    $liveDayStartNs = Convert-DateToUnixNs -LocalDate $liveDay
    $liveDayEndNs = Convert-DateToUnixNs -LocalDate ($liveDay.AddDays(1))
    $jitterMinutes = @(0, 5, -5, 10, -10)

    $seedRows = 0
    for ($i = 0; $i -lt 5; $i++) {
        $day = $weekMonday.AddDays($i)
        $base = $day.Date.AddHours(9).AddMinutes($jitterMinutes[$i])
        $tag = "train-$i"
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$tag-o-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime $base) -ValueJson (New-FocusRow -Marker $marker -App 'outlook.exe' -Title 'Inbox - Outlook')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$tag-x-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($base.AddMinutes(2))) -ValueJson (New-FocusRow -Marker $marker -App 'excel.exe' -Title 'report.xlsx - Excel')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$tag-t-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($base.AddMinutes(7))) -ValueJson (New-FocusRow -Marker $marker -App 'teams.exe' -Title 'Chat - Teams')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$tag-i-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($base.AddMinutes(9))) -ValueJson (New-IdleRow -Marker $marker)
        $seedRows += 4
    }

    $liveBase = $liveDay.Date.AddHours(9)
    $notepadBase = $liveDay.Date.AddHours(11)
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-o-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime $liveBase) -ValueJson (New-FocusRow -Marker $marker -App 'outlook.exe' -Title 'Inbox - Outlook')
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-x-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(2))) -ValueJson (New-FocusRow -Marker $marker -App 'excel.exe' -Title 'report.xlsx - Excel')
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-i-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(4))) -ValueJson (New-IdleRow -Marker $marker)
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-n-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime $notepadBase) -ValueJson (New-FocusRow -Marker $marker -App 'notepad.exe' -Title 'untitled - Notepad')
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-ni-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($notepadBase.AddMinutes(3))) -ValueJson (New-IdleRow -Marker $marker)
    $seedRows += 5

    $timelineMatches = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'timeline_search' -Arguments @{
        text = $marker
        start_ts_ns = $trainingStartNs
        end_ts_ns = $liveDayEndNs
        limit = 100
    }
    $timelineMatchCount = @($timelineMatches.matches).Count
    Assert-True ($timelineMatchCount -eq $seedRows) "timeline_search match count mismatch: $timelineMatchCount expected=$seedRows"

    $segmentedInitial = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_segment' -Arguments @{
        start_ts_ns = $trainingStartNs
        end_ts_ns = $liveDayEndNs
    }
    Assert-True ([int]$segmentedInitial.episodes_written -eq 18) "initial episodes_written mismatch: $($segmentedInitial.episodes_written)"

    $mined = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_mine' -Arguments @{
        start_ts_ns = $trainingStartNs
        end_ts_ns = $trainingEndNs
        min_support_days = 3
    }
    Assert-True ([int]$mined.routines_written -eq 1) "routines_written mismatch: $($mined.routines_written)"
    $routine = @($mined.routines)[0]
    $routineId = [string]$routine.routine_id
    $minedConfidence = [double]$routine.confidence
    $stepApps = @($routine.steps | ForEach-Object { [string]$_.app })
    Assert-True (($stepApps -join ',') -eq 'outlook.exe,excel.exe,teams.exe') "routine steps mismatch: $($stepApps -join ',')"
    Assert-True ([int]$routine.support_days -eq 5) "support_days mismatch: $($routine.support_days)"
    Assert-True ([int]$routine.opportunity_days -eq 5) "opportunity_days mismatch: $($routine.opportunity_days)"

    $confirm = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_update' -Arguments @{
        routine_id = $routineId
        action = 'confirm'
        note = 'issue857 diagnostic confirmed mined routine library'
    }
    Assert-True ([string]$confirm.lifecycle_after -eq 'confirmed') "confirm lifecycle_after=$($confirm.lifecycle_after)"

    $nowHappy = Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(6))
    $liveTimed = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_current' -Arguments @{ now_ts_ns = $nowHappy } -BudgetMs $LatencyBudgetMs
    $live = $liveTimed.result
    Assert-True ([int]$live.evaluated_routines -eq 1) "intent_current evaluated_routines mismatch: $($live.evaluated_routines)"
    Assert-True (@($live.candidates).Count -eq 1) "intent_current live candidate count mismatch: $(@($live.candidates).Count)"
    $top = @($live.candidates)[0]
    Assert-True ([string]$top.routine_id -eq $routineId) "top routine mismatch: $($top.routine_id)"
    Assert-True ([int]$top.matched_prefix_len -eq 2) "matched_prefix_len mismatch: $($top.matched_prefix_len)"
    Assert-True ([int]$top.total_steps -eq 3) "total_steps mismatch: $($top.total_steps)"
    Assert-True ([string]@($top.remaining_steps)[0].app -eq 'teams.exe') "remaining step mismatch"
    Assert-True ($top.schedule.dow_match -eq $true) 'schedule dow_match false'
    Assert-True ($top.schedule.within_tolerance -eq $true) 'schedule within_tolerance false'

    $matchedEpisodeId = [string]@($top.matched_steps)[0].episode_id
    $matchedEpisode = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_get' -Arguments @{
        episode_id = $matchedEpisodeId
        refs_limit = 20
    }
    Assert-True ([string]$matchedEpisode.episode.app -eq 'outlook.exe') "matched episode app mismatch: $($matchedEpisode.episode.app)"
    Assert-True (@($matchedEpisode.timeline_refs).Count -ge 1) "matched episode missing timeline refs"

    $unrelatedTimed = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_current' -Arguments @{ now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($notepadBase.AddMinutes(7))) } -BudgetMs $LatencyBudgetMs
    Assert-True (@($unrelatedTimed.result.candidates).Count -eq 0) "unrelated activity produced candidates"

    $staleTimed = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_current' -Arguments @{ now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddHours(6))) } -BudgetMs $LatencyBudgetMs
    Assert-True (@($staleTimed.result.candidates).Count -eq 0) "stale activity produced candidates"

    $subscription = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'subscribe' -Arguments @{
        kinds = @('intent-detected', 'intent-confirmed', 'intent-abandoned')
    }
    $subscriptionId = [string]$subscription.subscription_id
    Assert-True (-not [string]::IsNullOrWhiteSpace($subscriptionId)) 'subscription_id missing'

    $detectedTimed = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_detect_tick' -Arguments @{ now_ts_ns = $nowHappy; min_confidence = 0.1 } -BudgetMs $LatencyBudgetMs
    $detected = $detectedTimed.result
    Assert-True ([int]$detected.candidates -eq 1) "detected candidates mismatch: $($detected.candidates)"
    Assert-True ((Transition-Kinds $detected) -join ',' -eq 'detected') "detected transitions mismatch: $((Transition-Kinds $detected) -join ',')"
    Assert-True ([int]$detected.events_published -eq 1) "detected events_published mismatch: $($detected.events_published)"
    Assert-True ([int]$detected.events_matched_subscribers -ge 1) "detected event did not match subscriber"

    $abandonedTimed = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_detect_tick' -Arguments @{ now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(40))); min_confidence = 0.1 } -BudgetMs $LatencyBudgetMs
    $abandoned = $abandonedTimed.result
    Assert-True ([int]$abandoned.candidates -eq 0) "abandoned candidates mismatch: $($abandoned.candidates)"
    Assert-True ((Transition-Kinds $abandoned) -join ',' -eq 'abandoned') "abandoned transitions mismatch: $((Transition-Kinds $abandoned) -join ',')"
    Assert-True ([string]@($abandoned.transitions)[0].routine_id -eq $routineId) "abandoned routine mismatch"
    Assert-True ([int]@($abandoned.transitions)[0].matched_prefix_len -eq 2) "abandoned matched_prefix_len mismatch"
    Assert-True ([int]$abandoned.events_matched_subscribers -ge 1) "abandoned event did not match subscriber"

    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-t-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(7))) -ValueJson (New-FocusRow -Marker $marker -App 'teams.exe' -Title 'Chat - Teams')
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "live-ti-$marker" -TsNs (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(9))) -ValueJson (New-IdleRow -Marker $marker)
    $seedRows += 2

    $segmentedFinalDay = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_segment' -Arguments @{
        start_ts_ns = $liveDayStartNs
        end_ts_ns = $liveDayEndNs
    }
    Assert-True ([int]$segmentedFinalDay.episodes_written -eq 4) "final day episodes_written mismatch: $($segmentedFinalDay.episodes_written)"
    $timelineMatchesFinal = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'timeline_search' -Arguments @{
        text = $marker
        start_ts_ns = $trainingStartNs
        end_ts_ns = $liveDayEndNs
        limit = 100
    }
    $timelineMatchCountFinal = @($timelineMatchesFinal.matches).Count
    Assert-True ($timelineMatchCountFinal -eq $seedRows) "final timeline_search match count mismatch: $timelineMatchCountFinal expected=$seedRows"

    $confirmedTimed = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_detect_tick' -Arguments @{ now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(12))); min_confidence = 0.1 } -BudgetMs $LatencyBudgetMs
    $confirmed = $confirmedTimed.result
    Assert-True ([int]$confirmed.candidates -eq 1) "confirmed candidates mismatch: $($confirmed.candidates)"
    Assert-True ((Transition-Kinds $confirmed) -join ',' -eq 'detected,confirmed') "confirmed transitions mismatch: $((Transition-Kinds $confirmed) -join ',')"
    $confirmTransition = @($confirmed.transitions | Where-Object { [string]$_.kind -eq 'confirmed' } | Select-Object -First 1)
    Assert-True ([string]$confirmTransition.routine_id -eq $routineId) "confirmed routine mismatch"
    Assert-True ([int]$confirmTransition.matched_prefix_len -eq 3) "confirmed matched_prefix_len mismatch"
    Assert-True ([int]$confirmed.events_published -eq 2) "confirmed events_published mismatch: $($confirmed.events_published)"
    Assert-True ([int]$confirmed.events_matched_subscribers -ge 2) "confirmed events did not match subscriber"

    $silent = Invoke-McpToolTimed -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'intent_detect_tick' -Arguments @{ now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddHours(1))); min_confidence = 0.1 } -BudgetMs $LatencyBudgetMs
    Assert-True ([int]$silent.result.candidates -eq 0) "silent candidates mismatch: $($silent.result.candidates)"
    Assert-True (@($silent.result.transitions).Count -eq 0) "completed stale routine emitted transitions"

    $feedbackAbandoned = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_feedback' -Arguments @{
        routine_id = $routineId
        outcome = 'abandoned'
        note = 'issue857 diagnostic abandon provenance after intent-abandoned'
        now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(41)))
    }
    Assert-True ([int]$feedbackAbandoned.abandon_count -eq 1) "abandon_count mismatch: $($feedbackAbandoned.abandon_count)"
    Assert-True ($feedbackAbandoned.suppressed -eq $false) 'abandoned feedback should not suppress'
    Assert-True ([math]::Abs([double]$feedbackAbandoned.effective_confidence - $minedConfidence) -lt 0.000001) "abandoned effective_confidence moved unexpectedly"

    $feedbackDeclined = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_feedback' -Arguments @{
        routine_id = $routineId
        outcome = 'declined'
        note = 'issue857 diagnostic decline lowers confidence and starts cooldown'
        now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(42)))
    }
    Assert-True ([int]$feedbackDeclined.decline_count -eq 1) "decline_count mismatch: $($feedbackDeclined.decline_count)"
    Assert-True ([int]$feedbackDeclined.consecutive_declines -eq 1) "consecutive_declines mismatch"
    Assert-True ($feedbackDeclined.suppressed -eq $true) 'decline should suppress'
    Assert-True ([int64]$feedbackDeclined.cooldown_remaining_secs -eq 3600) "cooldown mismatch: $($feedbackDeclined.cooldown_remaining_secs)"
    Assert-True ([double]$feedbackDeclined.effective_confidence -lt $minedConfidence) "decline did not lower effective confidence"

    $feedbackAccepted = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_feedback' -Arguments @{
        routine_id = $routineId
        outcome = 'accepted'
        note = 'issue857 diagnostic accept raises confidence and clears cooldown'
        now_ts_ns = (Convert-LocalDateTimeToUnixNs -LocalDateTime ($liveBase.AddMinutes(43)))
    }
    Assert-True ([int]$feedbackAccepted.accept_count -eq 1) "accept_count mismatch: $($feedbackAccepted.accept_count)"
    Assert-True ([int]$feedbackAccepted.decline_count -eq 1) "accepted readback lost decline count"
    Assert-True ([int]$feedbackAccepted.abandon_count -eq 1) "accepted readback lost abandon count"
    Assert-True ([int]$feedbackAccepted.consecutive_declines -eq 0) "accepted did not reset consecutive_declines"
    Assert-True ($feedbackAccepted.suppressed -eq $false) 'accepted should clear suppression'
    Assert-True ([double]$feedbackAccepted.effective_confidence -gt [double]$feedbackDeclined.effective_confidence) "accept did not raise effective confidence"
    Assert-True ([double]$feedbackAccepted.effective_confidence -lt $minedConfidence) "feedback lower bound should keep effective confidence below mined confidence after one decline"

    $inspectAfterFeedback = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_inspect' -Arguments @{ routine_id = $routineId }
    Assert-True (@($inspectAfterFeedback.state.feedback_events).Count -eq 3) "feedback_events count mismatch: $(@($inspectAfterFeedback.state.feedback_events).Count)"
    Assert-True ([int]$inspectAfterFeedback.state.accept_count -eq 1) 'inspect accept_count mismatch'
    Assert-True ([int]$inspectAfterFeedback.state.decline_count -eq 1) 'inspect decline_count mismatch'
    Assert-True ([int]$inspectAfterFeedback.state.abandon_count -eq 1) 'inspect abandon_count mismatch'

    $episodesFinal = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_list' -Arguments @{
        start_ts_ns = $trainingStartNs
        end_ts_ns = $liveDayEndNs
        limit = 100
    }
    $episodeListCount = @($episodesFinal.episodes).Count
    Assert-True ($episodeListCount -eq 19) "final episode_list count mismatch: $episodeListCount"

    $cancel = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'subscribe_cancel' -Arguments @{
        subscription_id = $subscriptionId
    }
    Assert-True ($cancel.cancelled -eq $true) 'subscribe_cancel did not cancel'

    $storage = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'storage_inspect' -Arguments @{}
    $sessionEnd = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'session_end' -Arguments @{}

    $transcript = [ordered]@{
        issue = 857
        stamp = $stamp
        marker = $marker
        daemon = [ordered]@{
            pid = $health.pid
            bind = $bind
            db_path = $dbPath
            tool_count = $health.tool_count
            tool_surface_sha256 = $health.tool_surface_sha256
        }
        session_id = $sessionId
        latency_budget_ms = $LatencyBudgetMs
        tool_surface = [ordered]@{
            visible_count = @($toolNames).Count
            required_tools_present = $true
        }
        seed = [ordered]@{
            marker_rows = $seedRows
            initial_timeline_search_matches = $timelineMatchCount
            final_timeline_search_matches = $timelineMatchCountFinal
            training_monday = $weekMonday.ToString('yyyy-MM-dd')
            live_day = $liveDay.ToString('yyyy-MM-dd')
        }
        segmentation = [ordered]@{
            initial_episodes_written = $segmentedInitial.episodes_written
            final_day_episodes_written = $segmentedFinalDay.episodes_written
            final_episode_list_count = $episodeListCount
        }
        mining = [ordered]@{
            routine_id = $routineId
            steps = $stepApps
            support_days = $routine.support_days
            opportunity_days = $routine.opportunity_days
            confidence = $minedConfidence
            schedule_label = $routine.schedule_label
        }
        intent_current = [ordered]@{
            live_latency_ms = $liveTimed.elapsed_ms
            live_candidates = @($live.candidates).Count
            matched_prefix_len = $top.matched_prefix_len
            remaining_step = @($top.remaining_steps)[0].app
            unrelated_latency_ms = $unrelatedTimed.elapsed_ms
            unrelated_candidates = @($unrelatedTimed.result.candidates).Count
            stale_latency_ms = $staleTimed.elapsed_ms
            stale_candidates = @($staleTimed.result.candidates).Count
            matched_episode_id = $matchedEpisodeId
            matched_episode_ref_count = @($matchedEpisode.timeline_refs).Count
        }
        bus = [ordered]@{
            subscription_id = $subscriptionId
            detected_latency_ms = $detectedTimed.elapsed_ms
            detected_transitions = (Transition-Kinds $detected)
            detected_events_published = $detected.events_published
            detected_matched_subscribers = $detected.events_matched_subscribers
            abandoned_latency_ms = $abandonedTimed.elapsed_ms
            abandoned_transitions = (Transition-Kinds $abandoned)
            abandoned_events_published = $abandoned.events_published
            abandoned_matched_subscribers = $abandoned.events_matched_subscribers
            confirmed_latency_ms = $confirmedTimed.elapsed_ms
            confirmed_transitions = (Transition-Kinds $confirmed)
            confirmed_events_published = $confirmed.events_published
            confirmed_matched_subscribers = $confirmed.events_matched_subscribers
            completed_stale_transitions = @($silent.result.transitions).Count
            subscription_cancelled = $cancel.cancelled
        }
        feedback = [ordered]@{
            mined_confidence = $minedConfidence
            abandoned_effective_confidence = $feedbackAbandoned.effective_confidence
            declined_effective_confidence = $feedbackDeclined.effective_confidence
            declined_suppressed = $feedbackDeclined.suppressed
            declined_cooldown_remaining_secs = $feedbackDeclined.cooldown_remaining_secs
            accepted_effective_confidence = $feedbackAccepted.effective_confidence
            accepted_suppressed = $feedbackAccepted.suppressed
            accept_count = $inspectAfterFeedback.state.accept_count
            decline_count = $inspectAfterFeedback.state.decline_count
            abandon_count = $inspectAfterFeedback.state.abandon_count
            feedback_events = @($inspectAfterFeedback.state.feedback_events).Count
        }
        physical_readback = [ordered]@{
            storage_cf_row_counts = $storage.cf_row_counts
            storage_cf_sizes = $storage.cf_sizes
        }
        session_end = $sessionEnd
    }

    $transcript | ConvertTo-Json -Depth 80
} finally {
    if ($sessionId) {
        Close-McpSession -Bind $bind -Token $token -SessionId $sessionId
    }
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        try { $process.WaitForExit(5000) | Out-Null } catch {}
    }
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
