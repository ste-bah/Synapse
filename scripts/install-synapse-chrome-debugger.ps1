param(
    [string]$SynapseNativeHostExe = "$env:USERPROFILE\.cargo\bin\synapse-chrome-native-host.exe",
    [string]$ExtensionId = "leoocgnkjnplbfdbklajepahofecgfbk",
    # Maintenance/self-heal entry point: run ONLY the one-way removal of any
    # debugger/nativeMessaging blockers Synapse wrote into the Chrome
    # ExtensionSettings policy, print the result, and exit.
    [switch]$RemoveExternalDebuggerPolicyOnly,
    # Emergency/operator opt-out. Default behavior shields the normal Chrome
    # profile from layout-shifting debugger/native-host popups by adding
    # Synapse-authored blocked_permissions entries for detected hazards.
    [switch]$PreserveExternalDebuggerExtensions
)

$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Security -ErrorAction SilentlyContinue

function ConvertTo-CompressedJson {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value,
        [int]$Depth = 12
    )
    ConvertTo-Json -InputObject $Value -Depth $Depth -Compress
}

function Get-RegistryAclDiagnostic {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $cursor = $Path
    while (-not [string]::IsNullOrWhiteSpace($cursor) -and -not (Test-Path -LiteralPath $cursor)) {
        if ($cursor -notmatch '^(.*)\\[^\\]+$') {
            break
        }
        $cursor = $Matches[1]
    }
    if ([string]::IsNullOrWhiteSpace($cursor)) {
        $cursor = $Path
    }
    try {
        $acl = Get-Acl -LiteralPath $cursor -ErrorAction Stop
        $access = @($acl.Access | ForEach-Object {
            [pscustomobject]@{
                identity = [string]$_.IdentityReference
                rights = [string]$_.RegistryRights
                type = [string]$_.AccessControlType
                inherited = [bool]$_.IsInherited
            }
        })
        return ConvertTo-CompressedJson -Value ([ordered]@{
            requested_path = $Path
            inspected_path = $cursor
            owner = [string]$acl.Owner
            access = $access
        }) -Depth 8
    } catch {
        return "requested_path=$Path inspected_path=$cursor acl_error=$($_.Exception.Message)"
    }
}

function Get-ChromePolicyRoot {
    param(
        [ValidateSet('HKCU', 'HKLM')]
        [string]$Hive
    )
    return "${Hive}:\Software\Policies\Google\Chrome"
}

function Get-ChromePolicyHiveCandidates {
    param(
        [ValidateSet('Auto', 'HKCU', 'HKLM')]
        [string]$Hive
    )

    if ($Hive -eq 'Auto') {
        return @('HKCU', 'HKLM')
    }
    return @($Hive)
}

function Read-ChromeExtensionSettingsPolicy {
    param(
        [ValidateSet('HKCU', 'HKLM')]
        [string]$Hive
    )

    $policyRoot = Get-ChromePolicyRoot -Hive $Hive
    if (-not (Test-Path -LiteralPath $policyRoot)) {
        return [ordered]@{}
    }
    $raw = (Get-ItemProperty -LiteralPath $policyRoot -Name ExtensionSettings -ErrorAction SilentlyContinue).ExtensionSettings
    if ([string]::IsNullOrWhiteSpace([string]$raw)) {
        return [ordered]@{}
    }
    try {
        $parsed = $raw | ConvertFrom-Json -ErrorAction Stop
    } catch {
        throw "SYNAPSE_CHROME_EXTENSION_SETTINGS_POLICY_INVALID hive=$Hive path=$policyRoot value_name=ExtensionSettings parse_error=$($_.Exception.Message) remediation=fix the existing Chrome ExtensionSettings policy JSON before Synapse can merge debugger/nativeMessaging blockers without overwriting policy state"
    }

    $settings = [ordered]@{}
    foreach ($property in $parsed.PSObject.Properties) {
        $entry = [ordered]@{}
        foreach ($entryProperty in $property.Value.PSObject.Properties) {
            if ($entryProperty.Value -is [System.Array]) {
                $entry[$entryProperty.Name] = @($entryProperty.Value)
            } else {
                $entry[$entryProperty.Name] = $entryProperty.Value
            }
        }
        $settings[$property.Name] = $entry
    }
    return $settings
}

# Exact blocked_install_message a previous Synapse version stamped onto every
# ExtensionSettings entry it authored. It is the ONLY reliable marker that lets
# the self-heal below distinguish Synapse-authored blockers from policy an
# enterprise admin or the user set themselves. Do not change this string: it
# must match byte-for-byte what shipped installers wrote, or old installs will
# not be healed.
$script:SynapseChromeBlockedInstallMessage = 'Synapse blocked this extension on this host because debugger/nativeMessaging permissions can surface Chrome debugger or native-host popups during background automation.'

function Remove-SynapseChromeExternalDebuggerPolicy {
    param(
        [string[]]$PreserveExtensionIds = @()
    )

    # Reversible self-heal. This only undoes debugger/nativeMessaging blockers
    # that Synapse wrote into Chrome ExtensionSettings, identified strictly by
    # the Synapse blocked_install_message marker. Entries Synapse did not author
    # are left byte-for-byte untouched. Best-effort per hive: a hive we cannot
    # write is reported with ACL evidence, never silently swallowed, and never
    # fatal.
    $preserveIds = @($PreserveExtensionIds | Where-Object {
        $_ -match '^[a-p]{32}$'
    } | Sort-Object -Unique)
    $results = @()
    foreach ($hive in (Get-ChromePolicyHiveCandidates -Hive 'Auto')) {
        $policyRoot = Get-ChromePolicyRoot -Hive $hive
        if (-not (Test-Path -LiteralPath $policyRoot)) {
            $results += [pscustomobject]@{ hive = $hive; path = $policyRoot; changed = $false; reason = 'policy_root_absent' }
            continue
        }
        $raw = (Get-ItemProperty -LiteralPath $policyRoot -Name ExtensionSettings -ErrorAction SilentlyContinue).ExtensionSettings
        if ([string]::IsNullOrWhiteSpace([string]$raw)) {
            $results += [pscustomobject]@{ hive = $hive; path = $policyRoot; changed = $false; reason = 'no_extension_settings' }
            continue
        }
        try {
            $settings = Read-ChromeExtensionSettingsPolicy -Hive $hive
        } catch {
            # A policy we cannot parse is NOT ours to rewrite; surface it loudly
            # and leave it intact rather than risk corrupting admin policy.
            $results += [pscustomobject]@{ hive = $hive; path = $policyRoot; changed = $false; reason = "parse_error:$($_.Exception.Message)" }
            continue
        }
        $changed = $false
        $cleaned = [ordered]@{}
        $removedEntries = @()
        $strippedEntries = @()
        $preservedEntries = @()
        foreach ($name in @($settings.Keys)) {
            $entry = $settings[$name]
            $isSynapseAuthored = ($entry -is [System.Collections.Specialized.OrderedDictionary]) -and
                $entry.Contains('blocked_install_message') -and
                ([string]$entry['blocked_install_message'] -eq $script:SynapseChromeBlockedInstallMessage)
            if (-not $isSynapseAuthored) {
                $cleaned[$name] = $entry
                continue
            }
            if ($preserveIds -contains $name) {
                $cleaned[$name] = $entry
                $preservedEntries += $name
                continue
            }
            $changed = $true
            $blocked = @()
            if ($entry.Contains('blocked_permissions')) {
                $blocked = @($entry['blocked_permissions'] | Where-Object { $_ -ne 'debugger' -and $_ -ne 'nativeMessaging' })
            }
            $entry.Remove('blocked_install_message')
            if ($blocked.Count -gt 0) {
                $entry['blocked_permissions'] = $blocked
            } elseif ($entry.Contains('blocked_permissions')) {
                $entry.Remove('blocked_permissions')
            }
            # Drop the entry entirely only if Synapse's blockers were all it held;
            # otherwise preserve whatever else (e.g. an admin installation_mode).
            if ($entry.Keys.Count -gt 0) {
                $cleaned[$name] = $entry
                $strippedEntries += $name
            } else {
                $removedEntries += $name
            }
        }
        if (-not $changed) {
            $reason = if ($preservedEntries.Count -gt 0) {
                'only_current_synapse_popup_shields_present'
            } else {
                'no_synapse_authored_blocks'
            }
            $results += [pscustomobject]@{
                hive = $hive
                path = $policyRoot
                changed = $false
                reason = $reason
                preserved_entries = @($preservedEntries)
            }
            continue
        }
        try {
            if ($cleaned.Keys.Count -gt 0) {
                $json = ConvertTo-CompressedJson -Value $cleaned
                New-ItemProperty -LiteralPath $policyRoot -Name ExtensionSettings -PropertyType String -Value $json -Force | Out-Null
            } else {
                Remove-ItemProperty -LiteralPath $policyRoot -Name ExtensionSettings -ErrorAction Stop
            }
            $results += [pscustomobject]@{
                hive = $hive
                path = $policyRoot
                changed = $true
                reason = 'removed_synapse_authored_blocks'
                removed_entries = @($removedEntries)
                stripped_entries = @($strippedEntries)
                preserved_entries = @($preservedEntries)
                extension_settings_remaining = ($cleaned.Keys.Count -gt 0)
            }
        } catch {
            $results += [pscustomobject]@{
                hive = $hive
                path = $policyRoot
                changed = $false
                reason = "write_failed:$($_.Exception.Message)"
                acl_detail = (Get-RegistryAclDiagnostic -Path $policyRoot)
            }
        }
    }
    return @($results)
}

function Set-SynapseChromeExternalDebuggerPolicy {
    param(
        [object[]]$Extensions
    )

    $hazards = @($Extensions | Where-Object {
        $id = [string]$_.extension_id
        $id -match '^[a-p]{32}$'
    } | Sort-Object extension_id -Unique)

    $policyRoot = Get-ChromePolicyRoot -Hive 'HKCU'
    if ($hazards.Count -eq 0) {
        return [pscustomobject]@{
            hive = 'HKCU'
            path = $policyRoot
            changed = $false
            reason = 'no_debugger_or_native_hazards'
            shielded_entries = @()
            unchanged_entries = @()
        }
    }

    try {
        if (-not (Test-Path -LiteralPath $policyRoot)) {
            New-Item -Path $policyRoot -Force | Out-Null
        }
        $settings = Read-ChromeExtensionSettingsPolicy -Hive 'HKCU'
    } catch {
        $acl = Get-RegistryAclDiagnostic -Path $policyRoot
        return [pscustomobject]@{
            hive = 'HKCU'
            path = $policyRoot
            changed = $false
            reason = 'external_popup_shield_write_denied_requires_bridge_management'
            warning_code = 'SYNAPSE_CHROME_POLICY_POPUP_SHIELD_WRITE_DENIED'
            blocking = $true
            phase = 'read_or_create'
            error = $_.Exception.Message
            acl = $acl
            remediation = 'repair HKCU\Software\Policies\Google\Chrome ACL or run from elevated PowerShell so Synapse can apply the external debugger/nativeMessaging popup shield; otherwise the installed Synapse Chrome Bridge must suppress the hazard through chrome.management or normal bridge commands fail closed before touching Chrome'
            shielded_entries = @()
            unchanged_entries = @()
        }
    }
    $changed = $false
    $shieldedEntries = @()
    $unchangedEntries = @()

    foreach ($hazard in $hazards) {
        $id = [string]$hazard.extension_id
        $existing = $null
        if ($settings.Contains($id)) {
            $existing = $settings[$id]
        }
        if (-not ($existing -is [System.Collections.Specialized.OrderedDictionary])) {
            $existing = [ordered]@{}
        }

        $blocked = @()
        if ($existing.Contains('blocked_permissions')) {
            $blocked = @($existing['blocked_permissions'])
        }
        $nextBlocked = @($blocked + @('debugger', 'nativeMessaging') | Where-Object {
            -not [string]::IsNullOrWhiteSpace([string]$_)
        } | Sort-Object -Unique)

        $beforeBlocked = @($blocked | Sort-Object -Unique)
        $beforeMessage = $null
        if ($existing.Contains('blocked_install_message')) {
            $beforeMessage = [string]$existing['blocked_install_message']
        }

        $existing['blocked_permissions'] = $nextBlocked
        $existing['blocked_install_message'] = $script:SynapseChromeBlockedInstallMessage
        $settings[$id] = $existing

        if ((($beforeBlocked -join ',') -ne ($nextBlocked -join ',')) -or
            ($beforeMessage -ne $script:SynapseChromeBlockedInstallMessage)) {
            $changed = $true
            $shieldedEntries += [pscustomobject]@{
                extension_id = $id
                name = [string]$hazard.name
                active_api = @($hazard.active_api)
                granted_api = @($hazard.granted_api)
                manifest_api = @($hazard.manifest_api)
                hazard_api = @($hazard.hazard_api)
                runtime_enabled = [bool]$hazard.runtime_enabled
                source = [string]$hazard.source
            }
        } else {
            $unchangedEntries += $id
        }
    }

    if ($changed) {
        $json = ConvertTo-CompressedJson -Value $settings
        try {
            New-ItemProperty -LiteralPath $policyRoot -Name ExtensionSettings -PropertyType String -Value $json -Force | Out-Null
        } catch {
            $acl = Get-RegistryAclDiagnostic -Path $policyRoot
            return [pscustomobject]@{
                hive = 'HKCU'
                path = $policyRoot
                changed = $false
                reason = 'external_popup_shield_write_denied_requires_bridge_management'
                warning_code = 'SYNAPSE_CHROME_POLICY_POPUP_SHIELD_WRITE_DENIED'
                blocking = $true
                phase = 'write_extension_settings'
                error = $_.Exception.Message
                acl = $acl
                remediation = 'repair HKCU\Software\Policies\Google\Chrome ACL or run from elevated PowerShell so Synapse can apply the external debugger/nativeMessaging popup shield; otherwise the installed Synapse Chrome Bridge must suppress the hazard through chrome.management or normal bridge commands fail closed before touching Chrome'
                shielded_entries = @($shieldedEntries)
                unchanged_entries = @($unchangedEntries)
            }
        }
    }

    [pscustomobject]@{
        hive = 'HKCU'
        path = $policyRoot
        changed = $changed
        reason = if ($changed) { 'synapse_authored_popup_shield_applied' } else { 'synapse_authored_popup_shield_already_present' }
        shielded_entries = @($shieldedEntries)
        unchanged_entries = @($unchangedEntries)
    }
}

if ($RemoveExternalDebuggerPolicyOnly) {
    $cleanup = Remove-SynapseChromeExternalDebuggerPolicy -PreserveExtensionIds @($ExtensionId)
    [pscustomobject]@{
        ok = $true
        mode = 'chrome_policy_cleanup_only'
        chrome_policy_cleanup = $cleanup
    }
    exit 0
}

function Get-SynapseNativeHostRegistryTargets {
    param(
        [Parameter(Mandatory = $true)]
        [string]$HostName
    )

    @(
        [pscustomobject]@{
            hive = 'HKCU'
            registry_view = '64'
            path = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$HostName"
        },
        [pscustomobject]@{
            hive = 'HKLM'
            registry_view = '64'
            path = "HKLM:\Software\Google\Chrome\NativeMessagingHosts\$HostName"
        },
        [pscustomobject]@{
            hive = 'HKCU'
            registry_view = '32'
            path = "HKCU:\Software\Wow6432Node\Google\Chrome\NativeMessagingHosts\$HostName"
        },
        [pscustomobject]@{
            hive = 'HKLM'
            registry_view = '32'
            path = "HKLM:\Software\Wow6432Node\Google\Chrome\NativeMessagingHosts\$HostName"
        }
    )
}

function Read-SynapseNativeHostRegistryEntries {
    param(
        [Parameter(Mandatory = $true)]
        [string]$HostName
    )

    $entries = @()
    foreach ($target in (Get-SynapseNativeHostRegistryTargets -HostName $HostName)) {
        if (-not (Test-Path -LiteralPath $target.path)) {
            continue
        }
        try {
            $key = Get-Item -LiteralPath $target.path -ErrorAction Stop
            $manifestPath = [string]$key.GetValue('')
            $entries += [pscustomobject]@{
                hive = $target.hive
                registry_view = $target.registry_view
                path = $target.path
                manifest_path = $manifestPath
                read_error = $null
            }
        } catch {
            $entries += [pscustomobject]@{
                hive = $target.hive
                registry_view = $target.registry_view
                path = $target.path
                manifest_path = $null
                read_error = $_.Exception.Message
            }
        }
    }
    return @($entries)
}

function Remove-SynapseNativeHostRegistryEntries {
    param(
        [Parameter(Mandatory = $true)]
        [string]$HostName
    )

    $before = @(Read-SynapseNativeHostRegistryEntries -HostName $HostName)
    $removed = @()
    $failures = @()

    foreach ($entry in $before) {
        try {
            Remove-Item -LiteralPath $entry.path -Force -ErrorAction Stop
            $removed += [pscustomobject]@{
                hive = $entry.hive
                registry_view = $entry.registry_view
                path = $entry.path
                manifest_path = $entry.manifest_path
            }
        } catch {
            $failures += [pscustomobject]@{
                hive = $entry.hive
                registry_view = $entry.registry_view
                path = $entry.path
                manifest_path = $entry.manifest_path
                error = $_.Exception.Message
                acl_detail = Get-RegistryAclDiagnostic -Path $entry.path
            }
        }
    }

    $after = @(Read-SynapseNativeHostRegistryEntries -HostName $HostName)
    if ($failures.Count -gt 0 -or $after.Count -gt 0) {
        $detail = ConvertTo-CompressedJson -Value ([ordered]@{
            host_name = $HostName
            before = $before
            removed = $removed
            after = $after
            failures = $failures
        }) -Depth 10
        throw "SYNAPSE_CHROME_NATIVE_HOST_REGISTRY_REMOVE_FAILED_ALL_HIVES detail=$detail remediation=normal bridge must not leave Synapse nativeMessaging host registration in any Chrome lookup hive because Chrome can launch visible native-host wrappers; remove the listed registry keys from a principal that can write them and rerun this verifier"
    }

    [pscustomobject]@{
        host_name = $HostName
        before = $before
        removed = $removed
        after = $after
    }
}

if ($ExtensionId -notmatch '^[a-p]{32}$') {
    throw "SYNAPSE_CHROME_EXTENSION_ID_INVALID extension_id=$ExtensionId remediation=Chrome extension IDs are 32 lowercase characters in the range a-p; refusing to inspect profiles with an ambiguous extension identity"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$extensionDir = Join-Path $repoRoot 'extensions\synapse-chrome-debugger'
$manifestPath = Join-Path $extensionDir 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "SYNAPSE_CHROME_EXTENSION_MANIFEST_MISSING path=$manifestPath"
}
$extensionManifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
$requiredPermissions = @($extensionManifest.permissions)
$optionalPermissions = @($extensionManifest.optional_permissions)
$hostPermissions = @($extensionManifest.host_permissions)
if ($optionalPermissions -contains 'debugger') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_DEBUGGER_PERMISSION_FORBIDDEN path=$manifestPath remediation=the normal end-user bridge must be debugger-free; Chrome's debugger infobar changes viewport/layout and breaks coordinate truth"
}
if ($requiredPermissions -contains 'debugger') {
    throw "SYNAPSE_CHROME_EXTENSION_DEBUGGER_PERMISSION_FORBIDDEN path=$manifestPath remediation=the normal end-user bridge must not request debugger; use raw CDP from a dedicated Synapse-launched automation profile started with --silent-debugger-extension-api for debugger-backed work"
}
if ($requiredPermissions -contains 'nativeMessaging') {
    throw "SYNAPSE_CHROME_EXTENSION_NATIVE_MESSAGING_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use direct localhost HTTP registration plus WebSocket command delivery; nativeMessaging can launch a visible cmd.exe wrapper on Windows"
}
if ($optionalPermissions -contains 'nativeMessaging') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_NATIVE_MESSAGING_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must not request nativeMessaging"
}
if (-not ($requiredPermissions -contains 'management')) {
    throw "SYNAPSE_CHROME_EXTENSION_MANAGEMENT_PERMISSION_REQUIRED path=$manifestPath remediation=normal end-user bridge must request chrome.management so it can disable external debugger/nativeMessaging extensions or fail closed with exact readback before any browser command"
}
if (-not ($requiredPermissions -contains 'alarms')) {
    throw "SYNAPSE_CHROME_EXTENSION_ALARMS_PERMISSION_MISSING path=$manifestPath remediation=normal end-user bridge requires chrome.alarms so an MV3 service worker suspended after daemon restart can wake and re-register without foreground Chrome automation"
}
if ($optionalPermissions -contains 'alarms') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_ALARMS_PERMISSION_FORBIDDEN path=$manifestPath remediation=alarms must be a required permission for deterministic bridge wake/readback, not an optional runtime prompt"
}
if ($hostPermissions -notcontains 'http://127.0.0.1:7700/*') {
    throw "SYNAPSE_CHROME_EXTENSION_LOCALHOST_PERMISSION_MISSING path=$manifestPath remediation=normal bridge requires host_permissions http://127.0.0.1:7700/* for direct daemon registration and message posting"
}

$nativeRoot = Join-Path $env:APPDATA 'synapse\chrome-debugger'
New-Item -ItemType Directory -Force -Path $nativeRoot | Out-Null

$hostName = 'com.synapse.chrome_debugger'
$hostManifestPath = Join-Path $nativeRoot "$hostName.json"
$registryPath = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$hostName"
$nativeHostRegistryCleanup = Remove-SynapseNativeHostRegistryEntries -HostName $hostName
if (Test-Path -LiteralPath $hostManifestPath -PathType Leaf) {
    Remove-Item -LiteralPath $hostManifestPath -Force
}
if (Test-Path -LiteralPath $hostManifestPath -PathType Leaf) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_MANIFEST_REMOVE_FAILED path=$hostManifestPath remediation=normal bridge must use direct localhost WebSocket command delivery only"
}

$chromeProcesses = @(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" -ErrorAction SilentlyContinue | ForEach-Object {
    $commandLine = [string]$_.CommandLine
    $hasRemoteDebuggingPipe = $commandLine -match '(^|\s)--remote-debugging-pipe(\s|=|$)'
    $hasRemoteDebuggingPort = $commandLine -match '(^|\s)--remote-debugging-port(\s|=|$)'
    $hasSilentDebuggerSwitch = $commandLine -match '(^|\s)--silent-debugger-extension-api(\s|=|$)'
    $hasAutomationControlledFlag = $commandLine -match '(^|\s)--disable-blink-features=([^\s]*,)?AutomationControlled(,|\s|$)'
    $hasMsPlaywrightMcpDir = $commandLine -match 'ms-playwright-mcp'
    $layoutInfobarReasons = @()
    if ($hasAutomationControlledFlag) {
        $layoutInfobarReasons += 'unsupported_flag_disable_blink_features_automation_controlled'
    }
    if (($hasRemoteDebuggingPipe -or $hasRemoteDebuggingPort) -and -not $hasSilentDebuggerSwitch) {
        $layoutInfobarReasons += 'remote_debugging_without_silent_debugger_extension_api'
    }
    if ($hasMsPlaywrightMcpDir -and $hasAutomationControlledFlag) {
        $layoutInfobarReasons += 'headed_ms_playwright_mcp_layout_banner'
    }
    [pscustomobject]@{
        pid = [int]$_.ProcessId
        parent_pid = [int]$_.ParentProcessId
        creation_date = [string]$_.CreationDate
        command_line_readable = -not [string]::IsNullOrWhiteSpace($commandLine)
        has_silent_debugger_switch = $hasSilentDebuggerSwitch
        has_remote_debugging_pipe = $hasRemoteDebuggingPipe
        has_remote_debugging_port = $hasRemoteDebuggingPort
        has_automation_controlled_flag = $hasAutomationControlledFlag
        has_ms_playwright_mcp_dir = $hasMsPlaywrightMcpDir
        layout_infobar_risk = ($layoutInfobarReasons.Count -gt 0)
        layout_infobar_reasons = $layoutInfobarReasons
    }
})

$chromeUserDataRoot = Join-Path $env:LOCALAPPDATA 'Google\Chrome\User Data'
$profileDirs = @()
$synapseChromeProfileReadback = @()
$staleSynapseActivePermissions = @()
$staleSynapseGrantedPermissions = @()
$externalDebuggerOrNativeExtensions = @()
$externalDisabledDebuggerOrNativeExtensions = @()
$externalDebuggerExtensions = @()
function Get-ChromeExtensionRuntimeState {
    param(
        [Parameter(Mandatory = $true)]
        $Setting
    )

    $state = $null
    if ($Setting.PSObject.Properties.Name -contains 'state' -and $null -ne $Setting.state) {
        $state = [int]$Setting.state
    }
    $activeBit = $null
    if ($Setting.PSObject.Properties.Name -contains 'active_bit') {
        $activeBit = [bool]$Setting.active_bit
    }
    $disableReasons = @()
    if ($Setting.PSObject.Properties.Name -contains 'disable_reasons' -and $null -ne $Setting.disable_reasons) {
        $disableReasons = @($Setting.disable_reasons)
    }
    # Chromium persists extension state as DISABLED=0, ENABLED=1. Stale
    # permission rows can remain without state; the live chrome.management
    # bridge readback is the stronger authority for enabled hazards.
    $runtimeEnabled = (($state -eq 1) -and ($disableReasons.Count -eq 0))

    [pscustomobject]@{
        state = $state
        active_bit = $activeBit
        disable_reasons = $disableReasons
        runtime_enabled = $runtimeEnabled
    }
}

function Test-ExternalPopupRiskEnabled {
    param(
        [Parameter(Mandatory = $true)]
        [object]$RuntimeState,
        [Parameter(Mandatory = $true)]
        [bool]$HasActiveOrManifestHazard,
        [Parameter(Mandatory = $true)]
        [bool]$HasGrantedHazard
    )

    if ($RuntimeState.disable_reasons.Count -gt 0 -or $RuntimeState.state -eq 0) {
        return $false
    }
    if ($RuntimeState.state -eq 1) {
        return ($HasActiveOrManifestHazard -or $HasGrantedHazard)
    }
    # Stale granted-only residue is advisory, but an external active/manifest
    # debugger permission with no disable reason can still surface Chrome's
    # layout-shifting "started debugging this browser" infobar.
    return $HasActiveOrManifestHazard
}
if (Test-Path -LiteralPath $chromeUserDataRoot -PathType Container) {
    $profileDirs = @(Get-ChildItem -LiteralPath $chromeUserDataRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -ne 'Snapshots' -and (
                (Test-Path -LiteralPath (Join-Path $_.FullName 'Preferences') -PathType Leaf) -or
                (Test-Path -LiteralPath (Join-Path $_.FullName 'Secure Preferences') -PathType Leaf)
            )
        })
    foreach ($profileDir in $profileDirs) {
        $extensionRuntimeById = @{}
        foreach ($prefFileName in @('Preferences', 'Secure Preferences')) {
            $prefPath = Join-Path $profileDir.FullName $prefFileName
            if (-not (Test-Path -LiteralPath $prefPath -PathType Leaf)) {
                continue
            }
            try {
                $pref = Get-Content -Raw -LiteralPath $prefPath | ConvertFrom-Json -ErrorAction Stop
            } catch {
                $synapseChromeProfileReadback += [pscustomobject]@{
                    profile = $profileDir.Name
                    pref_file = $prefFileName
                    path = $prefPath
                    parse_error = $_.Exception.Message
                }
                continue
            }
            if (-not $pref.extensions -or -not $pref.extensions.settings) {
                continue
            }
            foreach ($extensionProperty in $pref.extensions.settings.PSObject.Properties) {
                $setting = $extensionProperty.Value
                $runtimeState = Get-ChromeExtensionRuntimeState -Setting $setting
                if ($prefFileName -eq 'Preferences') {
                    $extensionRuntimeById[$extensionProperty.Name] = $runtimeState
                } elseif ($extensionRuntimeById.ContainsKey($extensionProperty.Name)) {
                    $runtimeState = $extensionRuntimeById[$extensionProperty.Name]
                }
                $activeApi = @()
                if ($setting.active_permissions -and $setting.active_permissions.api) {
                    $activeApi = @($setting.active_permissions.api)
                }
                $grantedApi = @()
                if ($setting.granted_permissions -and $setting.granted_permissions.api) {
                    $grantedApi = @($setting.granted_permissions.api)
                }
                $manifestApi = @()
                if ($setting.manifest -and $setting.manifest.permissions) {
                    $manifestApi = @($setting.manifest.permissions | Where-Object {
                        $_ -is [string]
                    })
                }
                if ($extensionProperty.Name -eq $ExtensionId) {
                    $synapseActiveHazardApi = @(
                        @($activeApi)
                        @($manifestApi)
                    ) | Where-Object {
                        $_ -eq 'debugger' -or $_ -eq 'nativeMessaging'
                    } | Sort-Object -Unique
                    $synapseGrantedHazardApi = @($grantedApi | Where-Object {
                        $_ -eq 'debugger' -or $_ -eq 'nativeMessaging'
                    } | Sort-Object -Unique)
                    $row = [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        path = $prefPath
                        manifest_path = $setting.path
                        active_api = $activeApi
                        granted_api = $grantedApi
                        manifest_api = $manifestApi
                        active_or_manifest_hazard_api = $synapseActiveHazardApi
                        granted_hazard_api = $synapseGrantedHazardApi
                        state = $runtimeState.state
                        active_bit = $runtimeState.active_bit
                        disable_reasons = $runtimeState.disable_reasons
                        runtime_enabled = $runtimeState.runtime_enabled
                    }
                    $synapseChromeProfileReadback += $row
                    if ($synapseActiveHazardApi.Count -gt 0) {
                        $staleSynapseActivePermissions += $row
                    } elseif ($synapseGrantedHazardApi.Count -gt 0) {
                        $staleSynapseGrantedPermissions += $row
                    }
                } else {
                    $activeOrManifestHazardApi = @(
                        @($activeApi)
                        @($manifestApi)
                    ) | Where-Object {
                        $_ -eq 'debugger' -or $_ -eq 'nativeMessaging'
                    } | Sort-Object -Unique
                    $grantedHazardApi = @($grantedApi | Where-Object {
                        $_ -eq 'debugger' -or $_ -eq 'nativeMessaging'
                    } | Sort-Object -Unique)
                    $hazardApi = @(
                        @($activeOrManifestHazardApi)
                        @($grantedApi)
                    ) | Where-Object {
                        $_ -eq 'debugger' -or $_ -eq 'nativeMessaging'
                    } | Sort-Object -Unique
                    if ($hazardApi.Count -eq 0) {
                        continue
                    }
                    $popupRiskEnabled = Test-ExternalPopupRiskEnabled `
                        -RuntimeState $runtimeState `
                        -HasActiveOrManifestHazard ($activeOrManifestHazardApi.Count -gt 0) `
                        -HasGrantedHazard ($grantedHazardApi.Count -gt 0)
                    $externalRow = [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        extension_id = $extensionProperty.Name
                        name = $setting.manifest.name
                        location = $setting.location
                        manifest_path = $setting.path
                        active_api = $activeApi
                        granted_api = $grantedApi
                        manifest_api = $manifestApi
                        active_or_manifest_hazard_api = $activeOrManifestHazardApi
                        granted_hazard_api = $grantedHazardApi
                        hazard_api = $hazardApi
                        state = $runtimeState.state
                        active_bit = $runtimeState.active_bit
                        disable_reasons = $runtimeState.disable_reasons
                        runtime_enabled = $runtimeState.runtime_enabled
                        popup_risk_enabled = $popupRiskEnabled
                        risk_basis = if ($popupRiskEnabled) {
                            if ($runtimeState.state -eq 1) {
                                'state_enabled_hazard'
                            } elseif ($activeOrManifestHazardApi.Count -gt 0) {
                                'active_or_manifest_hazard_without_disable_reason'
                            } else {
                                'state_enabled_granted_hazard'
                            }
                        } else {
                            'disabled_or_granted_only_stale'
                        }
                    }
                    if ($popupRiskEnabled) {
                        $externalDebuggerOrNativeExtensions += $externalRow
                        if ($hazardApi -contains 'debugger') {
                            $externalDebuggerExtensions += $externalRow
                        }
                    } else {
                        $externalDisabledDebuggerOrNativeExtensions += $externalRow
                    }
                }
            }
        }
    }
}
$activeChromeProfile = $null
$chromeLocalStatePath = Join-Path $chromeUserDataRoot 'Local State'
if (Test-Path -LiteralPath $chromeLocalStatePath -PathType Leaf) {
    try {
        $chromeLocalState = Get-Content -Raw -LiteralPath $chromeLocalStatePath | ConvertFrom-Json -ErrorAction Stop
        if ($chromeLocalState.profile -and $chromeLocalState.profile.last_used) {
            $activeChromeProfile = [string]$chromeLocalState.profile.last_used
        }
    } catch {
        $activeChromeProfile = $null
    }
}
$synapseChromeInstalledProfiles = @(
    $synapseChromeProfileReadback |
        Where-Object { $_.PSObject.Properties.Name -contains 'manifest_path' } |
        ForEach-Object { [string]$_.profile } |
        Sort-Object -Unique
)
$synapseChromeActiveProfileInstalled = $null
if (-not [string]::IsNullOrWhiteSpace($activeChromeProfile)) {
    $synapseChromeActiveProfileInstalled = @($synapseChromeInstalledProfiles) -contains $activeChromeProfile
}
$synapseChromeProfileInstallReason = if ($profileDirs.Count -eq 0) {
    'no_profile_dirs'
} elseif ($synapseChromeInstalledProfiles.Count -eq 0) {
    'extension_id_absent_from_preferences_and_secure_preferences'
} elseif ($synapseChromeActiveProfileInstalled -eq $false) {
    'active_profile_missing_extension'
} else {
    'extension_profile_row_present'
}
$synapseChromeProfileInstallState = [pscustomobject]@{
    scanned = $true
    installed = ($synapseChromeInstalledProfiles.Count -gt 0)
    chrome_user_data_root = $chromeUserDataRoot
    profile_count = $profileDirs.Count
    installed_profile_count = $synapseChromeInstalledProfiles.Count
    installed_profiles = @($synapseChromeInstalledProfiles)
    active_profile = $activeChromeProfile
    active_profile_installed = $synapseChromeActiveProfileInstalled
    reason = $synapseChromeProfileInstallReason
    cdp_bridge_reload_can_install_absent_extension = $false
    remediation = 'run scripts\install-synapse-chrome-debugger.ps1 to validate files and policies, then load extensions\synapse-chrome-debugger as an unpacked extension in the already-open Chrome profile; cdp_bridge_reload can only reload an already-registered bridge host and cannot install an absent Chrome extension'
}
$externalNativeMessagingProcesses = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object {
        $_.CommandLine -match 'chrome\.nativeMessaging' -and
        $_.CommandLine -notmatch [regex]::Escape($ExtensionId)
    } |
    Select-Object ProcessId, ParentProcessId, Name, ExecutablePath, CommandLine,
        @{Name = 'ExtensionId'; Expression = {
            if ($_.CommandLine -match 'chrome-extension://([a-p]{32})') {
                $Matches[1]
            } else {
                $null
            }
        }})

$externalNativeMessagingProcessRows = @($externalNativeMessagingProcesses | ForEach-Object {
    if ($_.ExtensionId -match '^[a-p]{32}$') {
        [pscustomobject]@{
            profile = 'process'
            pref_file = 'process'
            extension_id = $_.ExtensionId
            name = 'external nativeMessaging process'
            location = $null
            manifest_path = $null
            active_api = @('nativeMessaging')
            active_bit = $null
            disable_reasons = @()
            runtime_enabled = $true
            source = 'native_messaging_process'
        }
    }
})
$externalLayoutInfobarProcesses = @($chromeProcesses | Where-Object {
    $_.layout_infobar_risk
} | ForEach-Object {
    [pscustomobject]@{
        pid = $_.pid
        parent_pid = $_.parent_pid
        reasons = @($_.layout_infobar_reasons)
        has_remote_debugging_pipe = $_.has_remote_debugging_pipe
        has_remote_debugging_port = $_.has_remote_debugging_port
        has_silent_debugger_switch = $_.has_silent_debugger_switch
        has_automation_controlled_flag = $_.has_automation_controlled_flag
        has_ms_playwright_mcp_dir = $_.has_ms_playwright_mcp_dir
    }
})
$allExternalDebuggerOrNativeExtensions = @(
    @($externalDebuggerOrNativeExtensions | ForEach-Object {
        $_ | Add-Member -NotePropertyName source -NotePropertyValue 'chrome_profile_active' -Force -PassThru
    })
    @($externalDisabledDebuggerOrNativeExtensions | ForEach-Object {
        $_ | Add-Member -NotePropertyName source -NotePropertyValue 'chrome_profile_disabled_or_inactive' -Force -PassThru
    })
    @($externalNativeMessagingProcessRows)
)
$externalHazardExtensionIds = @(
    @($allExternalDebuggerOrNativeExtensions | ForEach-Object { $_.extension_id })
    @($externalNativeMessagingProcesses | ForEach-Object { $_.ExtensionId })
) | Where-Object { $_ -match '^[a-p]{32}$' } | Sort-Object -Unique

$synapseSelfShieldRow = [pscustomobject]@{
    extension_id = $ExtensionId
    name = 'Synapse Chrome Bridge'
    active_api = @()
    granted_api = @()
    manifest_api = @()
    hazard_api = @('debugger', 'nativeMessaging')
    runtime_enabled = $true
    source = 'synapse_self_bridge_invariant'
}

# External extensions and native-messaging hosts that use debugger/nativeMessaging
# can surface layout-changing Chrome popups independently of Synapse's tabs-only
# bridge. Setup tries to apply a reversible HKCU ExtensionSettings shield for
# those permissions by default. If this host denies that policy write, the
# installed bridge must suppress the hazards through chrome.management or normal
# tabs/scripting commands fail closed before queueing any Chrome command.
#
# As a one-way remediation we remove stale Synapse-authored blockers from prior
# builds, but preserve the current self-shield for the Synapse extension ID. The
# current bridge does not request debugger/nativeMessaging, so blocking those
# permissions is harmless for the supported build and prevents an older loaded
# Synapse Chrome Bridge from retaining the capability that triggers Chrome's
# layout-shifting "started debugging this browser" banner.
$chromePolicyCleanup = Remove-SynapseChromeExternalDebuggerPolicy -PreserveExtensionIds @(
    @($externalHazardExtensionIds)
    $ExtensionId
)
$policyShieldExtensions = @($synapseSelfShieldRow)
if (-not $PreserveExternalDebuggerExtensions) {
    $policyShieldExtensions += @($allExternalDebuggerOrNativeExtensions)
}
$chromePolicyPopupShield = Set-SynapseChromeExternalDebuggerPolicy -Extensions $policyShieldExtensions

if ($staleSynapseActivePermissions.Count -gt 0) {
    $detail = [ordered]@{
        extension_id = $ExtensionId
        stale_active_permissions = $staleSynapseActivePermissions
        chrome_policy_popup_shield = $chromePolicyPopupShield
    } | ConvertTo-Json -Depth 8 -Compress
    throw "SYNAPSE_CHROME_EXTENSION_STALE_ACTIVE_DEBUGGER_PERMISSION extension_id=$ExtensionId detail=$detail remediation=Synapse attempted to apply/preserve the HKCU ExtensionSettings self-shield for debugger/nativeMessaging and included the physical policy write result in detail.chrome_policy_popup_shield; call cdp_bridge_reload through the real Synapse MCP tool when the live bridge advertises reloadSelf, otherwise keep normal browser commands failed closed until Chrome reloads the extension or restarts the already-open profile"
}

[pscustomobject]@{
    ok = $true
    native_host = $hostName
    native_manifest = $null
    registry_key = $registryPath
    binary = $null
    extension_id = $ExtensionId
    extension_dir = $extensionDir
    daemon_bridge_transport = 'direct_localhost_websocket'
    daemon_bridge_origin = "chrome-extension://$ExtensionId"
    bridge_self_reload_command = 'cdp_bridge_reload'
    bridge_build_id_expected = 'synapse-chrome-bridge-2026-06-21-check-uncheck-v1'
    bridge_build_sha256_expected = 'a848cc84f03897822f0c37cb428eae220ca62c57ed47a5bf25c7ff850b2f7dd4'
    bridge_required_capabilities = @('alarmReconnect', 'activateTab', 'closeTab', 'coordinateClick', 'domAction', 'externalPopupRiskSuppression', 'listTabs', 'navigateTab', 'openTab', 'pageVitals', 'reloadSelf', 'targetInfo', 'targetInfoPageText', 'typeActiveElement', 'setFieldValue')
    background_navigation_backend = 'chrome.tabs_plus_chrome.scripting_executeScript_for_typed_dom_actions_no_debugger_no_native_messaging_plus_chrome.management_external_popup_suppression'
    reconnect_driver = 'bounded_websocket_reconnect_with_chrome_alarms_mv3_wake'
    attach_popup_prevention = 'normal_bridge_debugger_free_no_chrome.debugger_permission_no_helper_windows_no_nativeMessaging_permission_plus_daemon_side_attach_disabled_for_debugger_commands'
    normal_bridge_attach_commands_available = $false
    normal_bridge_debugger_api_calls_present = $false
    expected_extension_id_guard_present = $true
    required_alarms_permission_present = ($requiredPermissions -contains 'alarms')
    recurring_wakeup_permission_present = ($requiredPermissions -contains 'alarms')
    required_debugger_permission_present = $false
    optional_debugger_permission_present = $false
    required_management_permission_present = ($requiredPermissions -contains 'management')
    required_native_messaging_permission_present = $false
    optional_native_messaging_permission_present = $false
    localhost_host_permission_present = $true
    native_host_registry_keys = @((Get-SynapseNativeHostRegistryTargets -HostName $hostName) | ForEach-Object { $_.path })
    native_host_registry_cleanup = $nativeHostRegistryCleanup
    native_host_registry_present = ($nativeHostRegistryCleanup.after.Count -gt 0)
    native_host_manifest_present = (Test-Path -LiteralPath $hostManifestPath)
    silent_debugger_switch_required_for_attach_commands = $false
    silent_debugger_switch = $null
    current_chrome_processes = $chromeProcesses
    chrome_policy_cleanup = $chromePolicyCleanup
    chrome_policy_popup_shield = $chromePolicyPopupShield
    external_popup_risk_blocks_popup_free_commands = ($allExternalDebuggerOrNativeExtensions.Count -gt 0)
    external_popup_risk_scope = 'runtime_bridge_management_suppression_or_fail_closed'
    external_popup_risk_block_reason = if ($allExternalDebuggerOrNativeExtensions.Count -gt 0) { 'external_debugger_or_native_hazards_require_chrome_management_suppression_or_policy_shield' } else { 'none' }
    external_popup_risk_bridge_management_required = ($allExternalDebuggerOrNativeExtensions.Count -gt 0)
    external_popup_risk_bridge_management_permission_present = ($requiredPermissions -contains 'management')
    synapse_chrome_profile_install_state = $synapseChromeProfileInstallState
    synapse_chrome_profile_readback = $synapseChromeProfileReadback
    synapse_stale_granted_permission_warning = [pscustomobject]@{
        warning = ($staleSynapseGrantedPermissions.Count -gt 0)
        scope = 'profile_granted_permissions_only_not_runtime_active'
        rows = $staleSynapseGrantedPermissions
    }
    external_hazard_extension_ids = $externalHazardExtensionIds
    external_debugger_or_native_extensions = $externalDebuggerOrNativeExtensions
    external_disabled_debugger_or_native_extensions = $externalDisabledDebuggerOrNativeExtensions
    external_debugger_extensions = $externalDebuggerExtensions
    external_native_messaging_processes = $externalNativeMessagingProcesses
    external_layout_infobar_processes = $externalLayoutInfobarProcesses
}
