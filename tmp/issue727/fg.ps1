Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class FGW {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageW(IntPtr h, int msg, IntPtr w, StringBuilder l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageLen(IntPtr h, int msg, IntPtr w, IntPtr l);
}
'@
$h = [FGW]::GetForegroundWindow()
$sb = New-Object System.Text.StringBuilder 512
[void][FGW]::GetWindowText($h, $sb, 512)
$obj = [ordered]@{ fg_hwnd = $h.ToInt64(); fg_hwnd_hex = ('0x' + $h.ToInt64().ToString('X')); fg_title = $sb.ToString() }
$obj | ConvertTo-Json -Compress
