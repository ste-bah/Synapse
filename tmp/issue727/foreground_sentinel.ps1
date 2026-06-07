Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Synapse727SentinelNative {
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@

$hwndPath = $env:SYNAPSE_727_SENTINEL_HWND_PATH
if ([string]::IsNullOrWhiteSpace($hwndPath)) {
    throw "SYNAPSE_727_SENTINEL_HWND_PATH is required"
}

$form = New-Object System.Windows.Forms.Form
$form.Text = "Synapse FSV 727 Foreground Sentinel"
$form.Name = "synapseFsv727ForegroundSentinel"
$form.StartPosition = "Manual"
$form.Location = New-Object System.Drawing.Point(-30000, -30000)
$form.Size = New-Object System.Drawing.Size(120, 60)
$form.TopMost = $false

$form.Add_Shown({
    Set-Content -LiteralPath $hwndPath -Value $form.Handle.ToInt64() -Encoding ASCII
    [Synapse727SentinelNative]::SetForegroundWindow($form.Handle) | Out-Null
})

[System.Windows.Forms.Application]::EnableVisualStyles()
[System.Windows.Forms.Application]::Run($form)
