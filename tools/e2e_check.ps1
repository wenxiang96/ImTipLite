# ImTipLite end-to-end verification (synchronous). UTF-8 with BOM.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'ImTipLite.exe'
$pname = [System.IO.Path]::GetFileNameWithoutExtension($exe)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Collections.Generic;
public static class W {
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static void Tap(byte vk) { keybd_event(vk,0,0,UIntPtr.Zero); System.Threading.Thread.Sleep(70); keybd_event(vk,0,2,UIntPtr.Zero); }
  public static string Text(IntPtr h) { var sb = new StringBuilder(512); GetWindowTextW(h, sb, 512); return sb.ToString(); }
  public static string Cls(IntPtr h) { var sb = new StringBuilder(256); GetClassNameW(h, sb, 256); return sb.ToString(); }
  public static List<IntPtr> WindowsOfPid(uint pid) {
    var list = new List<IntPtr>();
    EnumWindows(delegate(IntPtr h, IntPtr p) {
      uint wp; GetWindowThreadProcessId(h, out wp);
      if (wp == pid) list.Add(h);
      return true;
    }, IntPtr.Zero);
    return list;
  }
  public static string Describe(IntPtr h) {
    RECT r; GetWindowRect(h, out r);
    return string.Format("cls={0} vis={1} title={2} rect={3},{4} {5}x{6}", Cls(h), IsWindowVisible(h), Text(h), r.Left, r.Top, r.Right-r.Left, r.Bottom-r.Top);
  }
}
'@

$log  = Join-Path $env:TEMP 'imtip_dbg.log'
$dump = Join-Path $env:TEMP 'imtip_dump.bmp'
foreach ($f in @($log,$dump)) { if (Test-Path $f) { Remove-Item $f -Force } }
$env:IMTIP_DEBUG='1'; $env:IMTIP_DUMP='1'

Write-Output ("EXE=" + $exe)

# 清理上次残留, 否则单实例会直接退出
Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 900

# --- 0) 输入目标窗口(独立进程里的 WinForms 文本框) ---
$target = Start-Process powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'text_target.ps1') -PassThru
$tgt = [IntPtr]::Zero
for ($i=0; $i -lt 30; $i++) {
  Start-Sleep -Milliseconds 400
  $tgt = [W]::FindWindow($null, 'ImTipTestTarget')
  if ($tgt -ne [IntPtr]::Zero) { break }
}
Write-Output ("TARGET=0x{0:X}" -f [int64]$tgt)

# --- 1) 单实例 ---
$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 5
$app.Refresh()
Write-Output ("APP_PID=" + $app.Id + " EXITED=" + $app.HasExited + " COUNT=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
Write-Output ("MAIN_WINDOW: " + [W]::Describe([IntPtr]$app.MainWindowHandle))

function SettingsWindow([int]$procId) {
  foreach ($h in [W]::WindowsOfPid([uint32]$procId)) {
    if ([W]::Cls($h) -eq 'Tauri Window') { return $h }
  }
  return [IntPtr]::Zero
}
$settings = SettingsWindow $app.Id
Write-Output ("SETTINGS_HWND=0x{0:X} VISIBLE={1}" -f [int64]$settings, [W]::IsWindowVisible($settings))
Write-Output ("SETTINGS_DESC: " + [W]::Describe($settings))

# --- 2) 第二个实例: 立即退出且只剩 1 个进程 ---
$app2 = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 4
$app2.Refresh()
Write-Output ("SECOND_EXITED=" + $app2.HasExited + " COUNT=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
Write-Output ("AFTER_SECOND SETTINGS_VISIBLE=" + [W]::IsWindowVisible($settings))

# --- 3) 关窗不退出, 再启动可唤回 ---
[W]::PostMessage($settings, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep -Seconds 2
Write-Output ("AFTER_CLOSE visible=" + [W]::IsWindowVisible($settings) + " count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
$reopen = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 4
$reopen.Refresh()
Write-Output ("REOPEN second_exited=" + $reopen.HasExited + " count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count + " settings_visible=" + [W]::IsWindowVisible($settings))

# --- 4) 触发提示: Shift / CapsLock ---
if ($tgt -ne [IntPtr]::Zero) { [W]::SetForegroundWindow($tgt) | Out-Null; Start-Sleep -Milliseconds 700 }
function Shot($name) { if (Test-Path $dump) { Copy-Item $dump (Join-Path $root $name) -Force } }
function Tip { $h=[W]::FindWindow('ImTipLiteTip',$null); $v=$false
  if ($h -ne [IntPtr]::Zero) { $v=[W]::IsWindowVisible($h) }
  [pscustomobject]@{h=$h;v=$v} }

foreach ($sec in @(@{k=0xA0;n='SHIFT1'}, @{k=0x14;n='CAPS'}, @{k=0x14;n='CAPS_OFF'}, @{k=0xA0;n='SHIFT2'})) {
  [W]::Tap([byte]$sec.k)
  Start-Sleep -Milliseconds 1000
  $t = Tip
  Write-Output ("{0}: visible={1} hwnd=0x{2:X}" -f $sec.n, $t.v, [int64]$t.h)
  Shot ("_shot_" + $sec.n + ".bmp")
  Start-Sleep -Milliseconds 200
}

Write-Output '--- LOG (trigger/draw) ---'
if (Test-Path $log) {
  Get-Content $log | Where-Object { $_ -match 'show_tip|trigger key|draw text|render failed|ULW failed|empty|state:' } | Select-Object -Last 20 | ForEach-Object { Write-Output $_ }
} else { Write-Output 'NO_LOG' }

Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
if ($target) { Stop-Process -Id $target.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 1000
Write-Output ("FINAL_COUNT=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
