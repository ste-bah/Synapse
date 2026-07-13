# Supporting diagnostic only. Its output is supporting diagnostic evidence only.
# This script does not perform or accept Full State Verification (FSV). Under
# AGENTS.md D1, an agent must perform FSV manually
# through the strict production MCP client and independently read each physical
# Source of Truth before and after the trigger.
param(
    [string]$SynapseMcpExe = (Join-Path $PSScriptRoot '..\..\target\release\synapse-mcp.exe'),
    [string]$ProfileDir = "$env:USERPROFILE\.cargo\bin\profiles",
    [string]$TokenPath = "$env:APPDATA\synapse\token.txt",
    [int]$StartupTimeoutSeconds = 20
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Die($Message) { throw "[issue852-diagnostic] $Message" }

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) { Die $Message }
}

function ConvertTo-JsonCompact {
    param([Parameter(Mandatory = $true)]$Value)
    $Value | ConvertTo-Json -Depth 80 -Compress
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
        clientInfo = [ordered]@{ name = 'issue852-diagnostic'; version = '1' }
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
            pid = 852
            hwnd = 852
            source = 'issue852-diagnostic'
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
            source = 'issue852-diagnostic'
            diagnostic_marker = $Marker
        }
    }
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
$marker = "issue852-diagnostic-$stamp"
$bind = Get-FreeLoopbackBind
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "synapse-issue852-diagnostic-$stamp"
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
    foreach ($requiredTool in @('storage_put_probe_rows', 'timeline_search', 'episode_segment', 'episode_list', 'episode_get', 'routine_mine', 'routine_list', 'routine_inspect', 'routine_update', 'routine_label_export', 'timeline_digest', 'storage_inspect', 'session_end')) {
        Assert-True ($toolNames -contains $requiredTool) "required tool missing from temp daemon surface: $requiredTool"
    }

    $weekMonday = Get-LastFullWeekMonday
    $rangeStartNs = Convert-DateToUnixNs -LocalDate $weekMonday
    $rangeEndNs = Convert-DateToUnixNs -LocalDate ($weekMonday.AddDays(7))
    $jitterMinutes = @(0, 5, -5, 10, -10)
    $seededRows = 0
    for ($i = 0; $i -lt 5; $i++) {
        $day = $weekMonday.AddDays($i)
        $base = Convert-LocalDateTimeToUnixNs -LocalDateTime ($day.Date.AddHours(9).AddMinutes($jitterMinutes[$i]))
        $dayTag = $day.ToString('yyyyMMdd')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$marker-$dayTag-outlook" -TsNs $base -ValueJson (New-FocusRow -Marker $marker -App 'outlook.exe' -Title 'Inbox - Outlook')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$marker-$dayTag-excel" -TsNs ($base + 2 * 60 * 1000000000) -ValueJson (New-FocusRow -Marker $marker -App 'excel.exe' -Title 'report.xlsx - Excel')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$marker-$dayTag-teams" -TsNs ($base + 7 * 60 * 1000000000) -ValueJson (New-FocusRow -Marker $marker -App 'teams.exe' -Title 'Chat - Teams')
        Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$marker-$dayTag-idle" -TsNs ($base + 9 * 60 * 1000000000) -ValueJson (New-IdleRow -Marker $marker)
        $seededRows += 4
    }
    $noiseBase = Convert-LocalDateTimeToUnixNs -LocalDateTime ($weekMonday.Date.AddHours(14))
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$marker-noise-spotify" -TsNs $noiseBase -ValueJson (New-FocusRow -Marker $marker -App 'spotify.exe' -Title 'Spotify')
    Seed-TimelineRow -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Prefix "$marker-noise-idle" -TsNs ($noiseBase + 5 * 60 * 1000000000) -ValueJson (New-IdleRow -Marker $marker)
    $seededRows += 2

    $timelineMatches = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'timeline_search' -Arguments @{ text = $marker; limit = 100 }
    Assert-True (@($timelineMatches.matches).Count -eq $seededRows) "timeline search marker count mismatch expected=$seededRows actual=$(@($timelineMatches.matches).Count)"

    $segmented = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_segment' -Arguments @{
        start_ts_ns = $rangeStartNs
        end_ts_ns = $rangeEndNs
    } -TimeoutSec 120
    Assert-True ([int]$segmented.episodes_written -eq 16) "episodes_written mismatch: $($segmented.episodes_written)"
    Assert-True ([string]$segmented.stopped_because -eq 'range_complete') "episode_segment stopped_because=$($segmented.stopped_because)"

    $episodes = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_list' -Arguments @{
        start_ts_ns = $rangeStartNs
        end_ts_ns = $rangeEndNs
        limit = 100
    }
    Assert-True (@($episodes.episodes).Count -eq 16) "episode_list count mismatch: $(@($episodes.episodes).Count)"

    $dryMine = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_mine' -Arguments @{
        start_ts_ns = $rangeStartNs
        end_ts_ns = $rangeEndNs
        dry_run = $true
    } -TimeoutSec 120
    Assert-True ($dryMine.dry_run -eq $true) 'routine_mine dry_run flag missing'
    Assert-True ([int]$dryMine.routines_written -eq 0) "dry run wrote routines: $($dryMine.routines_written)"
    Assert-True (@($dryMine.routines).Count -eq 1) "dry run routine count mismatch: $(@($dryMine.routines).Count)"

    $mined = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_mine' -Arguments @{
        start_ts_ns = $rangeStartNs
        end_ts_ns = $rangeEndNs
    } -TimeoutSec 120
    Assert-True ([int]$mined.routines_written -eq 1) "routines_written mismatch: $($mined.routines_written)"
    Assert-True ([int]$mined.active_days -eq 5) "active_days mismatch: $($mined.active_days)"
    $routine = @($mined.routines)[0]
    $routineId = [string]$routine.routine_id
    $apps = @($routine.steps | ForEach-Object { [string]$_.app })
    Assert-True (($apps -join ',') -eq 'outlook.exe,excel.exe,teams.exe') "routine apps mismatch: $($apps -join ',')"
    Assert-True ([string]$routine.granularity -eq 'app_document') "granularity mismatch: $($routine.granularity)"
    Assert-True ([int]$routine.support_days -eq 5) "support_days mismatch: $($routine.support_days)"
    Assert-True ([int]$routine.opportunity_days -eq 5) "opportunity_days mismatch: $($routine.opportunity_days)"
    Assert-True ([int]$routine.occurrence_count -eq 5) "occurrence_count mismatch: $($routine.occurrence_count)"
    Assert-True ([double]$routine.confidence -gt 0.5) "confidence too low: $($routine.confidence)"
    Assert-True ([int]$routine.mean_minute_of_day -ge 535 -and [int]$routine.mean_minute_of_day -le 545) "mean_minute_of_day not near 09:00: $($routine.mean_minute_of_day)"
    Assert-True (@($routine.evidence).Count -eq 5) "evidence occurrence count mismatch: $(@($routine.evidence).Count)"

    $routineList = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_list' -Arguments @{ app = 'excel.exe'; limit = 10 }
    Assert-True ([int]$routineList.returned -eq 1) "routine_list returned mismatch: $($routineList.returned)"
    Assert-True ([string]@($routineList.entries)[0].routine_id -eq $routineId) "routine_list routine id mismatch"

    $inspectBefore = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_inspect' -Arguments @{ routine_id = $routineId }
    Assert-True ($inspectBefore.mined -eq $true) 'routine_inspect mined=false before update'
    Assert-True ([string]$inspectBefore.state.lifecycle -eq 'candidate') "initial lifecycle mismatch: $($inspectBefore.state.lifecycle)"

    $confirm = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_update' -Arguments @{
        routine_id = $routineId
        action = 'confirm'
        note = 'issue852 diagnostic confirm lifecycle readback'
    }
    Assert-True ([string]$confirm.lifecycle_after -eq 'confirmed') "confirm lifecycle_after=$($confirm.lifecycle_after)"

    $label = 'Morning report handoff diagnostic'
    $rename = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_update' -Arguments @{
        routine_id = $routineId
        action = 'rename'
        label = $label
        note = 'issue852 diagnostic labeling readback'
    }
    Assert-True ([string]$rename.label_after -eq $label) "rename label_after=$($rename.label_after)"

    $inspectAfter = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_inspect' -Arguments @{ routine_id = $routineId }
    Assert-True ([string]$inspectAfter.state.lifecycle -eq 'confirmed') "post lifecycle mismatch: $($inspectAfter.state.lifecycle)"
    Assert-True ([string]$inspectAfter.state.label -eq $label) "post label mismatch: $($inspectAfter.state.label)"
    Assert-True (@($inspectAfter.state.transitions).Count -ge 3) "transition audit too short: $(@($inspectAfter.state.transitions).Count)"

    $labelExport = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'routine_label_export' -Arguments @{
        routine_id = $routineId
        max_samples = 3
    }
    Assert-True ([string]$labelExport.current_label -eq $label) "label export current_label mismatch: $($labelExport.current_label)"
    Assert-True (@($labelExport.samples).Count -eq 3) "label export samples mismatch: $(@($labelExport.samples).Count)"
    Assert-True ([string]$labelExport.machine_identity -match 'outlook\.exe' -and [string]$labelExport.machine_identity -match 'excel\.exe' -and [string]$labelExport.machine_identity -match 'teams\.exe') "machine identity missing apps: $($labelExport.machine_identity)"
    Assert-True ([string]$labelExport.writeback_hint -match 'routine_update') 'label export writeback_hint missing routine_update'

    $evidenceEpisodeChecks = @()
    foreach ($evidence in @($routine.evidence)) {
        Assert-True (@($evidence.episode_ids).Count -eq 3) "evidence episode count mismatch for day $($evidence.day_start_ns)"
        foreach ($episodeId in @($evidence.episode_ids)) {
            $episode = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'episode_get' -Arguments @{
                episode_id = $episodeId
                refs_limit = 20
            }
            Assert-True ([string]$episode.episode.episode_id -eq [string]$episodeId) "episode_get id mismatch for $episodeId"
            Assert-True (@($episode.timeline_refs).Count -ge 1) "episode_get refs missing for $episodeId"
            $evidenceEpisodeChecks += [pscustomobject]@{
                episode_id = [string]$episodeId
                app = [string]$episode.episode.app
                duration_ms = [int64]$episode.episode.duration_ms
                timeline_ref_count = @($episode.timeline_refs).Count
            }
        }
    }

    $routineDayDigest = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'timeline_digest' -Arguments @{
        period = 'day'
        date = $weekMonday.AddDays(1).ToString('yyyy-MM-dd')
        top_n = 10
    }
    Assert-True ([int64]$routineDayDigest.active_ms -eq 540000) "routine day active_ms mismatch: $($routineDayDigest.active_ms)"
    Assert-True ([int]$routineDayDigest.episode_count -eq 3) "routine day episode_count mismatch: $($routineDayDigest.episode_count)"
    Assert-True (@($routineDayDigest.routines_touched).Count -eq 1) "routine day routines_touched mismatch: $(@($routineDayDigest.routines_touched).Count)"
    Assert-True ([string]@($routineDayDigest.routines_touched)[0].routine_id -eq $routineId) 'routine day digest routine id mismatch'
    Assert-True ([int64]@($routineDayDigest.routines_touched)[0].matched_episode_count -eq 3) "routine day matched_episode_count mismatch"

    $weekDigestAnchor = $weekMonday.AddDays(4)
    $weekDigest = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'timeline_digest' -Arguments @{
        period = 'week'
        date = $weekDigestAnchor.ToString('yyyy-MM-dd')
        top_n = 10
    }
    Assert-True ([string]$weekDigest.period -eq 'week') "week digest returned period=$($weekDigest.period)"
    Assert-True ([int64]$weekDigest.active_ms -eq 3000000) "week active_ms mismatch: $($weekDigest.active_ms)"
    Assert-True ([int]$weekDigest.episode_count -eq 16) "week episode_count mismatch: $($weekDigest.episode_count)"
    Assert-True (@($weekDigest.routines_touched).Count -eq 1) "week routines_touched mismatch: $(@($weekDigest.routines_touched).Count)"
    Assert-True ([int64]@($weekDigest.routines_touched)[0].matched_episode_count -eq 15) "week matched_episode_count mismatch"

    $storage = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'storage_inspect' -Arguments @{}
    $sessionEnd = Invoke-McpTool -Bind $bind -Token $token -SessionId $sessionId -NextId ([ref]$nextId) -Name 'session_end' -Arguments @{}

    $transcript = [ordered]@{
        issue = 852
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
        week = [ordered]@{
            monday = $weekMonday.ToString('yyyy-MM-dd')
            range_start_ns = $rangeStartNs
            range_end_ns = $rangeEndNs
        }
        tool_surface = [ordered]@{
            visible_count = @($toolNames).Count
            required_tools_present = $true
        }
        seed = [ordered]@{
            timeline_rows = $seededRows
            timeline_search_matches = @($timelineMatches.matches).Count
            planted_apps = @('outlook.exe', 'excel.exe', 'teams.exe')
            noise_app = 'spotify.exe'
        }
        segmentation = [ordered]@{
            episodes_written = $segmented.episodes_written
            episodes_deleted = $segmented.episodes_deleted
            days_processed = $segmented.days_processed
            stopped_because = $segmented.stopped_because
            episode_list_count = @($episodes.episodes).Count
        }
        mining = [ordered]@{
            dry_run_routines = @($dryMine.routines).Count
            dry_run_written = $dryMine.routines_written
            routines_written = $mined.routines_written
            routines_deleted = $mined.routines_deleted
            active_days = $mined.active_days
            candidates_evaluated = $mined.candidates_evaluated
            candidates_rejected_as_subpattern = $mined.candidates_rejected_as_subpattern
            clusters_rejected_low_support = $mined.clusters_rejected_low_support
            routine_id = $routineId
            steps = $apps
            granularity = $routine.granularity
            dow_class = $routine.dow_class
            schedule_label = $routine.schedule_label
            mean_minute_of_day = $routine.mean_minute_of_day
            tolerance_minutes = $routine.tolerance_minutes
            support_days = $routine.support_days
            opportunity_days = $routine.opportunity_days
            occurrence_count = $routine.occurrence_count
            confidence = $routine.confidence
            evidence_occurrences = @($routine.evidence).Count
        }
        query_lifecycle_labeling = [ordered]@{
            routine_list_returned = $routineList.returned
            inspect_before_lifecycle = $inspectBefore.state.lifecycle
            confirm_lifecycle_after = $confirm.lifecycle_after
            rename_label_after = $rename.label_after
            inspect_after_lifecycle = $inspectAfter.state.lifecycle
            inspect_after_label = $inspectAfter.state.label
            transition_count = @($inspectAfter.state.transitions).Count
            label_export_current_label = $labelExport.current_label
            label_export_samples = @($labelExport.samples).Count
            label_export_machine_identity = $labelExport.machine_identity
            label_export_writeback_hint = $labelExport.writeback_hint
        }
        evidence = [ordered]@{
            episode_get_checks = $evidenceEpisodeChecks
        }
        digest = [ordered]@{
            routine_day = [ordered]@{
                date = $weekMonday.AddDays(1).ToString('yyyy-MM-dd')
                active_ms = $routineDayDigest.active_ms
                episode_count = $routineDayDigest.episode_count
                routines_touched = @($routineDayDigest.routines_touched).Count
                matched_episode_count = @($routineDayDigest.routines_touched)[0].matched_episode_count
            }
            week = [ordered]@{
                anchor_date = $weekDigestAnchor.ToString('yyyy-MM-dd')
                active_ms = $weekDigest.active_ms
                episode_count = $weekDigest.episode_count
                routines_touched = @($weekDigest.routines_touched).Count
                matched_episode_count = @($weekDigest.routines_touched)[0].matched_episode_count
                days_covered = $weekDigest.days_covered
            }
        }
        physical_readback = [ordered]@{
            storage_cf_row_counts = $storage.cf_row_counts
            storage_cf_sizes = $storage.cf_sizes
        }
        session_end = $sessionEnd
    }
    $transcript | ConvertTo-Json -Depth 80
} finally {
    if (-not [string]::IsNullOrWhiteSpace($sessionId)) {
        try { Close-McpSession -Bind $bind -Token $token -SessionId $sessionId } catch {}
    }
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        try { $process.WaitForExit(5000) | Out-Null } catch {}
    }
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
