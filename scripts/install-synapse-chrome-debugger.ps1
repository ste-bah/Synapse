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
        $id -match '^[a-p]{32}$' -and $id -ne $ExtensionId
    } | Sort-Object extension_id -Unique)

    $policyRoot = Get-ChromePolicyRoot -Hive 'HKCU'
    if ($hazards.Count -eq 0) {
        return [pscustomobject]@{
            hive = 'HKCU'
            path = $policyRoot
            changed = $false
            reason = 'no_external_debugger_or_native_hazards'
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
        throw "SYNAPSE_CHROME_POLICY_POPUP_SHIELD_WRITE_DENIED hive=HKCU path=$policyRoot phase=read_or_create error=$($_.Exception.Message) acl=$acl remediation=run scripts\install-synapse-chrome-debugger.ps1 from an elevated PowerShell, or repair HKCU\Software\Policies\Google\Chrome ACL so the current user can write ExtensionSettings; until this succeeds Synapse must treat this normal Chrome profile as not popup-free"
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
            throw "SYNAPSE_CHROME_POLICY_POPUP_SHIELD_WRITE_DENIED hive=HKCU path=$policyRoot phase=write_extension_settings error=$($_.Exception.Message) acl=$acl remediation=run scripts\install-synapse-chrome-debugger.ps1 from an elevated PowerShell, or repair HKCU\Software\Policies\Google\Chrome ACL so the current user can write ExtensionSettings; until this succeeds Synapse must treat this normal Chrome profile as not popup-free"
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
    $cleanup = Remove-SynapseChromeExternalDebuggerPolicy
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
    [pscustomobject]@{
        pid = [int]$_.ProcessId
        parent_pid = [int]$_.ParentProcessId
        creation_date = [string]$_.CreationDate
        command_line_readable = -not [string]::IsNullOrWhiteSpace($commandLine)
        has_silent_debugger_switch = $commandLine -match '(^|\s)--silent-debugger-extension-api(\s|=|$)'
    }
})

$chromeUserDataRoot = Join-Path $env:LOCALAPPDATA 'Google\Chrome\User Data'
$synapseChromeProfileReadback = @()
$staleSynapseActivePermissions = @()
$externalDebuggerOrNativeExtensions = @()
$externalDisabledDebuggerOrNativeExtensions = @()
$externalDebuggerExtensions = @()
function Get-ChromeExtensionRuntimeState {
    param(
        [Parameter(Mandatory = $true)]
        $Setting
    )

    $activeBit = $null
    if ($Setting.PSObject.Properties.Name -contains 'active_bit') {
        $activeBit = [bool]$Setting.active_bit
    }
    $disableReasons = @()
    if ($Setting.PSObject.Properties.Name -contains 'disable_reasons' -and $null -ne $Setting.disable_reasons) {
        $disableReasons = @($Setting.disable_reasons)
    }
    $runtimeEnabled = $true
    # Chrome can leave active_bit=false on rows whose active_permissions still
    # include debugger/nativeMessaging. Disable reasons are the concrete SoT.
    if ($disableReasons.Count -gt 0) {
        $runtimeEnabled = $false
    }

    [pscustomobject]@{
        active_bit = $activeBit
        disable_reasons = $disableReasons
        runtime_enabled = $runtimeEnabled
    }
}
if (Test-Path -LiteralPath $chromeUserDataRoot -PathType Container) {
    $profileDirs = @(Get-ChildItem -LiteralPath $chromeUserDataRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ne 'Snapshots' })
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
                    $row = [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        path = $prefPath
                        manifest_path = $setting.path
                        active_api = $activeApi
                        granted_api = $grantedApi
                        manifest_api = $manifestApi
                        active_bit = $runtimeState.active_bit
                        disable_reasons = $runtimeState.disable_reasons
                        runtime_enabled = $runtimeState.runtime_enabled
                    }
                    $synapseChromeProfileReadback += $row
                    if ($activeApi -contains 'nativeMessaging') {
                        $staleSynapseActivePermissions += $row
                    }
                } else {
                    $hazardApi = @(
                        @($activeApi)
                        @($grantedApi)
                        @($manifestApi)
                    ) | Where-Object {
                        $_ -eq 'debugger' -or $_ -eq 'nativeMessaging'
                    } | Sort-Object -Unique
                    if ($hazardApi.Count -eq 0) {
                        continue
                    }
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
                        hazard_api = $hazardApi
                        active_bit = $runtimeState.active_bit
                        disable_reasons = $runtimeState.disable_reasons
                        runtime_enabled = $runtimeState.runtime_enabled
                    }
                    if ($runtimeState.runtime_enabled) {
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
if ($staleSynapseActivePermissions.Count -gt 0) {
    $detail = $staleSynapseActivePermissions | ConvertTo-Json -Depth 6 -Compress
    throw "SYNAPSE_CHROME_EXTENSION_STALE_ACTIVE_DEBUGGER_PERMISSION extension_id=$ExtensionId detail=$detail remediation=if daemon health shows the live bridge advertises reloadSelf, call cdp_bridge_reload through the real Synapse MCP tool; if the loaded worker predates reloadSelf, fail closed and let Chrome reload/restart the extension out-of-band instead of automating chrome://extensions foreground UI; the normal bridge must be active with tabs only before setup can pass"
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

# External extensions and native-messaging hosts that use debugger/nativeMessaging
# can surface layout-changing Chrome popups independently of Synapse's tabs-only
# bridge. Setup therefore applies a reversible HKCU ExtensionSettings shield for
# those permissions by default. The marker is Synapse-authored and can be removed
# with -RemoveExternalDebuggerPolicyOnly.
#
# As a one-way remediation we remove any debugger/nativeMessaging blockers that
# an earlier Synapse version wrote into Chrome ExtensionSettings, so running the
# latest build self-heals previously-affected machines.
$chromePolicyCleanup = Remove-SynapseChromeExternalDebuggerPolicy -PreserveExtensionIds $externalHazardExtensionIds
$chromePolicyPopupShield = if ($PreserveExternalDebuggerExtensions) {
    [pscustomobject]@{
        hive = 'HKCU'
        path = (Get-ChromePolicyRoot -Hive 'HKCU')
        changed = $false
        reason = 'preserve_external_debugger_extensions_requested'
        shielded_entries = @()
        unchanged_entries = @()
    }
} else {
    Set-SynapseChromeExternalDebuggerPolicy -Extensions $allExternalDebuggerOrNativeExtensions
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
    bridge_build_id_expected = 'synapse-chrome-bridge-2026-06-18-popup-free-tabs-v4'
    bridge_build_sha256_expected = '9a977c14087e4ecaf5c4807be7c164083366c2c7f30969f308caaf4fe43ed6f4'
    bridge_required_capabilities = @('alarmReconnect', 'activateTab', 'closeTab', 'domAction', 'navigateTab', 'openTab', 'pageVitals', 'reloadSelf', 'targetInfo', 'targetInfoPageText', 'typeActiveElement', 'setFieldValue')
    background_navigation_backend = 'chrome.tabs_plus_chrome.scripting_executeScript_for_typed_dom_actions_no_debugger_no_native_messaging'
    reconnect_driver = 'bounded_websocket_reconnect_with_chrome_alarms_mv3_wake'
    attach_popup_prevention = 'normal_bridge_debugger_free_no_chrome.debugger_permission_no_helper_windows_no_nativeMessaging_permission_plus_daemon_side_attach_disabled_for_debugger_commands'
    normal_bridge_attach_commands_available = $false
    normal_bridge_debugger_api_calls_present = $false
    expected_extension_id_guard_present = $true
    required_alarms_permission_present = ($requiredPermissions -contains 'alarms')
    recurring_wakeup_permission_present = ($requiredPermissions -contains 'alarms')
    required_debugger_permission_present = $false
    optional_debugger_permission_present = $false
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
    synapse_chrome_profile_readback = $synapseChromeProfileReadback
    external_hazard_extension_ids = $externalHazardExtensionIds
    external_debugger_or_native_extensions = $externalDebuggerOrNativeExtensions
    external_disabled_debugger_or_native_extensions = $externalDisabledDebuggerOrNativeExtensions
    external_debugger_extensions = $externalDebuggerExtensions
    external_native_messaging_processes = $externalNativeMessagingProcesses
}
