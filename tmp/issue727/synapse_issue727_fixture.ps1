Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Synapse727Native {
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@

$statePath = $env:SYNAPSE_727_STATE_PATH
if ([string]::IsNullOrWhiteSpace($statePath)) {
    throw "SYNAPSE_727_STATE_PATH is required"
}

function Write-State {
    param(
        [System.Windows.Forms.TextBox]$Normal,
        [System.Windows.Forms.TextBox]$ReadOnly,
        [System.Windows.Forms.TextBox]$Password,
        [System.Windows.Forms.TextBox]$Multiline
    )
    $state = [ordered]@{
        normal = $Normal.Text
        normal_len = $Normal.Text.Length
        read_only = $ReadOnly.Text
        read_only_len = $ReadOnly.Text.Length
        password_len = $Password.Text.Length
        multiline = $Multiline.Text
        multiline_len = $Multiline.Text.Length
        updated_at = [DateTime]::UtcNow.ToString("o")
    }
    $json = $state | ConvertTo-Json -Compress
    Set-Content -LiteralPath $statePath -Value $json -Encoding UTF8
}

$form = New-Object System.Windows.Forms.Form
$form.Text = "Synapse FSV 727 UIA"
$form.Name = "synapseFsv727Form"
$form.StartPosition = "Manual"
$form.Location = New-Object System.Drawing.Point(120, 120)
$form.Size = New-Object System.Drawing.Size(520, 280)
$form.TopMost = $false

$normal = New-Object System.Windows.Forms.TextBox
$normal.Name = "synapse727Normal"
$normal.AccessibleName = "synapse727Normal"
$normal.Location = New-Object System.Drawing.Point(20, 20)
$normal.Size = New-Object System.Drawing.Size(450, 24)

$readOnly = New-Object System.Windows.Forms.TextBox
$readOnly.Name = "synapse727ReadOnly"
$readOnly.AccessibleName = "synapse727ReadOnly"
$readOnly.ReadOnly = $true
$readOnly.Text = "LOCKED-727"
$readOnly.Location = New-Object System.Drawing.Point(20, 60)
$readOnly.Size = New-Object System.Drawing.Size(450, 24)

$password = New-Object System.Windows.Forms.TextBox
$password.Name = "synapse727Password"
$password.AccessibleName = "synapse727Password"
$password.UseSystemPasswordChar = $true
$password.Location = New-Object System.Drawing.Point(20, 100)
$password.Size = New-Object System.Drawing.Size(450, 24)

$multiline = New-Object System.Windows.Forms.TextBox
$multiline.Name = "synapse727Multiline"
$multiline.AccessibleName = "synapse727Multiline"
$multiline.Multiline = $true
$multiline.AcceptsReturn = $true
$multiline.ScrollBars = "Vertical"
$multiline.Location = New-Object System.Drawing.Point(20, 140)
$multiline.Size = New-Object System.Drawing.Size(450, 80)

$controls = @($normal, $readOnly, $password, $multiline)
foreach ($control in $controls) {
    $control.Add_TextChanged({ Write-State -Normal $normal -ReadOnly $readOnly -Password $password -Multiline $multiline })
    $form.Controls.Add($control)
}

$form.Add_Shown({
    [Synapse727Native]::ShowWindow($form.Handle, 4) | Out-Null
    Write-State -Normal $normal -ReadOnly $readOnly -Password $password -Multiline $multiline
})
$form.Add_FormClosed({
    Write-State -Normal $normal -ReadOnly $readOnly -Password $password -Multiline $multiline
})

[System.Windows.Forms.Application]::EnableVisualStyles()
[System.Windows.Forms.Application]::Run($form)
