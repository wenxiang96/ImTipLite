# Verify: icon color, close-button keeps running, re-open by launching exe again
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe  = Join-Path $root 'ImTipLite.exe'
$pname = [System.IO.Path]::GetFileNameWithoutExtension($exe)

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class U {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
'@

# --- 图标颜色 ---
$bmp = [System.Drawing.Icon]::ExtractAssociatedIcon($exe).ToBitmap()
$c = $bmp.GetPixel(16,16); $e = $bmp.GetPixel(3,3)
Write-Output ("ICON center=" + $c.ToString() + " edge=" + $e.ToString())
$bmp.Dispose()

Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 5
$app.Refresh()
Write-Output ("START count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count + " visible=" + [U]::IsWindowVisible($app.MainWindowHandle))

# --- 点关闭按钮(WM_CLOSE) ---
$hwnd = [U]::FindWindow($null, 'ImTipLite - 输入提示配置方案')
if ($hwnd -eq [IntPtr]::Zero) { $hwnd = $app.MainWindowHandle }
[U]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep -Seconds 2
$app.Refresh()
Write-Output ("AFTER_CLOSE count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count + " exited=" + $app.HasExited + " visible=" + [U]::IsWindowVisible($hwnd))

# --- 再次双击 exe: 应唤回设置窗口, 进程仍为 1 ---
$app2 = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 4
$app2.Refresh()
Write-Output ("REOPEN second_exited=" + $app2.HasExited + " count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count + " visible=" + [U]::IsWindowVisible($hwnd))

# --- 退出: 只剩一个进程且未复活 ---
Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 1000
Write-Output ("FINAL count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
