$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe  = Join-Path $root 'ImTipLite.exe'
$pname = [System.IO.Path]::GetFileNameWithoutExtension($exe)

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class U {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static string Cls(IntPtr h){ var s=new StringBuilder(256); GetClassNameW(h,s,256); return s.ToString(); }
  public static IntPtr SettingsOf(uint pid){
    IntPtr found = IntPtr.Zero;
    EnumWindows(delegate(IntPtr h, IntPtr p){
      uint wp; GetWindowThreadProcessId(h, out wp);
      if (wp == pid && Cls(h) == "Tauri Window") { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
'@

Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 900
$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 7

$h = [U]::SettingsOf([uint32]$app.Id)
Write-Output ("settings hwnd = 0x{0:X}" -f [int64]$h)
if ($h -eq [IntPtr]::Zero) { Write-Output 'FAIL 窗口未出现'; Stop-Process -Id $app.Id -Force; exit 1 }
[U]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1200
# 重新取坐标: 窗口可能被移动到前台过程中改变位置
$r = New-Object U+RECT
[U]::GetWindowRect($h, [ref]$r) | Out-Null
Write-Output ("rect = {0},{1} {2}x{3}" -f $r.Left,$r.Top,($r.Right-$r.Left),($r.Bottom-$r.Top))
# 让页面滚到底部, 使"字体"下拉进入视野 (WM_VSCROLL = 0x115, SB_BOTTOM = 7)
[U]::PostMessage($h, 0x0115, [IntPtr]7, [IntPtr]::Zero) | Out-Null
Start-Sleep -Milliseconds 1500
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [U]::PrintWindow($h, $hdc, 2)   # PW_RENDERFULLCONTENT: 抓 WebView2 内容需要
$g.ReleaseHdc($hdc)
$g.Dispose()
Write-Output ("PrintWindow = " + $ok)
$bmp.Save((Join-Path $root '_settings_shot.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output ("shot saved (" + $w + "x" + $ht + ")")

Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 700
Write-Output ("final count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
