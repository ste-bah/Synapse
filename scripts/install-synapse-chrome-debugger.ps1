param(
    [string]$SynapseNativeHostExe = "$env:USERPROFILE\.cargo\bin\synapse-chrome-native-host.exe",
    [string]$ExtensionId = "leoocgnkjnplbfdbklajepahofecgfbk",
    [switch]$ApplyExternalChromeDebuggerPolicy,
    [ValidateSet('HKCU', 'HKLM')]
    [string]$ChromePolicyHive = 'HKCU',
    [switch]$AllowExternalChromeDebuggerOrNativeMessaging
)

$ErrorActionPreference = 'Stop'

function ConvertTo-CompressedJson {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value,
        [int]$Depth = 12
    )
    $Value | ConvertTo-Json -Depth $Depth -Compress
}

function Get-ChromePolicyRoot {
    param(
        [ValidateSet('HKCU', 'HKLM')]
        [string]$Hive
    )
    return "${Hive}:\Software\Policies\Google\Chrome"
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

function Write-ChromeExternalDebuggerPolicy {
    param(
        [ValidateSet('HKCU', 'HKLM')]
        [string]$Hive,
        [string[]]$ExternalExtensionIds
    )

    $ids = @($ExternalExtensionIds | Where-Object { $_ -match '^[a-p]{32}$' } | Sort-Object -Unique)
    if ($ids.Count -eq 0) {
        return [pscustomobject]@{
            hive = $Hive
            path = Get-ChromePolicyRoot -Hive $Hive
            value_name = 'ExtensionSettings'
            blocked_extension_ids = @()
            readback_ok = $true
            policy_json = ConvertTo-CompressedJson -Value ([ordered]@{})
        }
    }

    $policyRoot = Get-ChromePolicyRoot -Hive $Hive
    try {
        New-Item -ItemType Directory -Force -Path $policyRoot | Out-Null
        $settings = Read-ChromeExtensionSettingsPolicy -Hive $Hive
        foreach ($id in $ids) {
            if (-not $settings.Contains($id)) {
                $settings[$id] = [ordered]@{}
            }
            $entry = $settings[$id]
            $blocked = @()
            if ($entry.Contains('blocked_permissions')) {
                $blocked = @($entry['blocked_permissions'])
            }
            $blocked = @($blocked + @('debugger', 'nativeMessaging') | Sort-Object -Unique)
            $entry['blocked_permissions'] = $blocked
            if (-not $entry.Contains('blocked_install_message')) {
                $entry['blocked_install_message'] = 'Synapse blocked this extension on this host because debugger/nativeMessaging permissions can surface Chrome debugger or native-host popups during background automation.'
            }
        }
        $json = ConvertTo-CompressedJson -Value $settings
        New-ItemProperty -LiteralPath $policyRoot -Name ExtensionSettings -PropertyType String -Value $json -Force | Out-Null
        $readback = Read-ChromeExtensionSettingsPolicy -Hive $Hive
        $missing = @()
        foreach ($id in $ids) {
            if (-not $readback.Contains($id)) {
                $missing += "${id}:missing_entry"
                continue
            }
            $blocked = @($readback[$id]['blocked_permissions'])
            foreach ($permission in @('debugger', 'nativeMessaging')) {
                if ($blocked -notcontains $permission) {
                    $missing += "${id}:missing_$permission"
                }
            }
        }
        if ($missing.Count -gt 0) {
            throw "SYNAPSE_CHROME_POLICY_READBACK_MISMATCH hive=$Hive path=$policyRoot missing=$($missing -join ',') remediation=Chrome ExtensionSettings policy write did not persist the required blocked_permissions"
        }
        return [pscustomobject]@{
            hive = $Hive
            path = $policyRoot
            value_name = 'ExtensionSettings'
            blocked_extension_ids = $ids
            readback_ok = $true
            policy_json = ConvertTo-CompressedJson -Value $readback
        }
    } catch {
        throw "SYNAPSE_CHROME_POLICY_REMEDIATION_WRITE_FAILED hive=$Hive path=$policyRoot blocked_extension_ids=$($ids -join ',') detail=$($_.Exception.Message) remediation=run setup from a principal that can write Chrome policy or disable/remove the named external Chrome extension, then refresh/restart Chrome and rerun this verifier"
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
if ($requiredPermissions -contains 'debugger') {
    throw "SYNAPSE_CHROME_EXTENSION_REQUIRED_DEBUGGER_PERMISSION_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use chrome.tabs without required debugger permission"
}
if ($optionalPermissions -contains 'debugger') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_DEBUGGER_PERMISSION_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use chrome.tabs without optional debugger permission; use raw CDP from a Synapse-launched automation profile for DOM/action CDP"
}
if ($requiredPermissions -contains 'nativeMessaging') {
    throw "SYNAPSE_CHROME_EXTENSION_NATIVE_MESSAGING_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use direct localhost HTTP registration plus WebSocket command delivery; nativeMessaging can launch a visible cmd.exe wrapper on Windows"
}
if ($optionalPermissions -contains 'nativeMessaging') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_NATIVE_MESSAGING_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must not request nativeMessaging"
}
if ($hostPermissions -notcontains 'http://127.0.0.1:7700/*') {
    throw "SYNAPSE_CHROME_EXTENSION_LOCALHOST_PERMISSION_MISSING path=$manifestPath remediation=normal bridge requires host_permissions http://127.0.0.1:7700/* for direct daemon registration and message posting"
}

$nativeRoot = Join-Path $env:APPDATA 'synapse\chrome-debugger'
New-Item -ItemType Directory -Force -Path $nativeRoot | Out-Null

$hostName = 'com.synapse.chrome_debugger'
$hostManifestPath = Join-Path $nativeRoot "$hostName.json"
$registryPath = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$hostName"
if (Test-Path -LiteralPath $registryPath) {
    Remove-Item -LiteralPath $registryPath -Force
}
if (Test-Path -LiteralPath $registryPath) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_REGISTRY_REMOVE_FAILED path=$registryPath remediation=normal bridge must not leave a nativeMessaging host registered because Chrome may launch cmd.exe as an intermediary"
}
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
        command_line_readable = -not [string]::IsNullOrWhiteSpace($commandLine)
        has_silent_debugger_switch = $commandLine -match '(^|\s)--silent-debugger-extension-api(\s|=|$)'
    }
})

$chromeUserDataRoot = Join-Path $env:LOCALAPPDATA 'Google\Chrome\User Data'
$synapseChromeProfileReadback = @()
$staleSynapseActivePermissions = @()
$externalDebuggerOrNativeExtensions = @()
$externalDebuggerExtensions = @()
if (Test-Path -LiteralPath $chromeUserDataRoot -PathType Container) {
    $profileDirs = @(Get-ChildItem -LiteralPath $chromeUserDataRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ne 'Snapshots' })
    foreach ($profileDir in $profileDirs) {
        foreach ($prefFileName in @('Secure Preferences', 'Preferences')) {
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
                $activeApi = @()
                if ($setting.active_permissions -and $setting.active_permissions.api) {
                    $activeApi = @($setting.active_permissions.api)
                }
                $grantedApi = @()
                if ($setting.granted_permissions -and $setting.granted_permissions.api) {
                    $grantedApi = @($setting.granted_permissions.api)
                }
                if ($extensionProperty.Name -eq $ExtensionId) {
                    $row = [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        path = $prefPath
                        manifest_path = $setting.path
                        active_api = $activeApi
                        granted_api = $grantedApi
                    }
                    $synapseChromeProfileReadback += $row
                    if ($activeApi -contains 'debugger' -or $activeApi -contains 'nativeMessaging') {
                        $staleSynapseActivePermissions += $row
                    }
                } elseif ($activeApi -contains 'debugger' -or $activeApi -contains 'nativeMessaging') {
                    $externalRow = [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        extension_id = $extensionProperty.Name
                        name = $setting.manifest.name
                        location = $setting.location
                        manifest_path = $setting.path
                        active_api = $activeApi
                    }
                    $externalDebuggerOrNativeExtensions += $externalRow
                    if ($activeApi -contains 'debugger') {
                        $externalDebuggerExtensions += $externalRow
                    }
                }
            }
        }
    }
}
if ($staleSynapseActivePermissions.Count -gt 0) {
    $detail = $staleSynapseActivePermissions | ConvertTo-Json -Depth 6 -Compress
    throw "SYNAPSE_CHROME_EXTENSION_STALE_ACTIVE_DEBUGGER_PERMISSION extension_id=$ExtensionId detail=$detail remediation=reload the unpacked Synapse Chrome Bridge from chrome://extensions or remove/re-add it; the normal bridge must be active with tabs only before setup can pass"
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

$externalHazardExtensionIds = @(
    @($externalDebuggerOrNativeExtensions | ForEach-Object { $_.extension_id })
    @($externalNativeMessagingProcesses | ForEach-Object { $_.ExtensionId })
) | Where-Object { $_ -match '^[a-p]{32}$' } | Sort-Object -Unique

$chromePolicyReadback = $null
if ($ApplyExternalChromeDebuggerPolicy -and $externalHazardExtensionIds.Count -gt 0) {
    $chromePolicyReadback = Write-ChromeExternalDebuggerPolicy -Hive $ChromePolicyHive -ExternalExtensionIds $externalHazardExtensionIds
}

if (-not $AllowExternalChromeDebuggerOrNativeMessaging -and
    ($externalDebuggerOrNativeExtensions.Count -gt 0 -or $externalNativeMessagingProcesses.Count -gt 0)) {
    $detail = [pscustomobject]@{
        external_debugger_or_native_extensions = $externalDebuggerOrNativeExtensions
        external_debugger_extensions = $externalDebuggerExtensions
        external_native_messaging_processes = $externalNativeMessagingProcesses
        external_hazard_extension_ids = $externalHazardExtensionIds
        current_chrome_processes = $chromeProcesses
        chrome_policy_readback = $chromePolicyReadback
        chrome_policy_remediation = 'HKCU/HKLM Chrome ExtensionSettings blocked_permissions=[debugger,nativeMessaging] for the offending extension, or disable/remove that extension, then refresh/restart Chrome and rerun this verifier'
    } | ConvertTo-Json -Depth 8 -Compress
    throw "SYNAPSE_CHROME_EXTERNAL_DEBUGGER_OR_NATIVE_SURFACE_PRESENT detail=$detail remediation=normal end-user systems cannot be certified banner-free while another active Chrome extension can call chrome.debugger or a live external native-messaging wrapper can surface a console/window; pass -AllowExternalChromeDebuggerOrNativeMessaging only for diagnostic attribution, never for popup-free acceptance"
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
    background_navigation_backend = 'chrome.tabs_no_debugger_permission_no_native_messaging'
    reconnect_driver = 'chrome.alarms_30s_direct_localhost_register'
    attach_popup_prevention = 'normal_bridge_tabs_only_no_debugger_api_no_nativeMessaging_permission_plus_daemon_side_attach_disabled'
    normal_bridge_attach_commands_available = $false
    normal_bridge_debugger_api_calls_present = $false
    required_alarms_permission_present = ($requiredPermissions -contains 'alarms')
    required_debugger_permission_present = $false
    optional_debugger_permission_present = $false
    required_native_messaging_permission_present = $false
    optional_native_messaging_permission_present = $false
    localhost_host_permission_present = $true
    native_host_registry_present = (Test-Path -LiteralPath $registryPath)
    native_host_manifest_present = (Test-Path -LiteralPath $hostManifestPath)
    silent_debugger_switch_required_for_attach_commands = $false
    silent_debugger_switch = $null
    current_chrome_processes = $chromeProcesses
    chrome_policy_scope = $ChromePolicyHive
    chrome_policy_readback = $chromePolicyReadback
    synapse_chrome_profile_readback = $synapseChromeProfileReadback
    external_hazard_extension_ids = $externalHazardExtensionIds
    external_debugger_or_native_extensions = $externalDebuggerOrNativeExtensions
    external_debugger_extensions = $externalDebuggerExtensions
    external_native_messaging_processes = $externalNativeMessagingProcesses
}
