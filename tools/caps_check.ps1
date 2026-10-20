# Verify CapsLock semantics as computed INSIDE ImTipLite.
# 关键: 每次按键后等足够久(覆盖 300ms 稳定期 + 2 次去抖轮询 + 渲染), 再读最后一条 trigger。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe  = Join-Path $root 'ImTipLite.exe'
$pname = [System.IO.Path]::GetFileNameWithoutExtension($exe)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class C {
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
  [DllImport("user32.dll")] public static extern short GetKeyState(int v);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  public static void Tap(byte vk){ keybd_event(vk,0,0,UIntPtr.Zero); System.Threading.Thread.Sleep(70); keybd_event(vk,0,2,UIntPtr.Zero); }
  public static int Caps(){
    IntPtr fg = GetForegroundWindow();
    uint tid = GetWindowThreadProcessId(fg, IntPtr.Zero);
    uint me = GetCurrentThreadId();
    if (tid != 0 && tid != me && AttachThreadInput(me, tid, true)) {
      int r = GetKeyState(0x14) & 1;
      AttachThreadInput(me, tid, false);
      return r;
    }
    return GetKeyState(0x14) & 1;
  }
}
'@

$failed = $false
$log = Join-Path $env:TEMP 'imtip_dbg.log'
if (Test-Path $log) { Remove-Item $log -Force }
$env:IMTIP_DEBUG='1'; $env:IMTIP_DUMP='1'

Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Get-Process powershell -ErrorAction SilentlyContinue |
  Where-Object { $_.MainWindowTitle -eq 'ImTipTestTarget' } | Stop-Process -Force
Start-Sleep -Milliseconds 900

$tgt = Start-Process powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'text_target.ps1') -PassThru
$w = [IntPtr]::Zero
for ($i=0; $i -lt 30; $i++) {
  Start-Sleep -Milliseconds 400
  $w = [C]::FindWindow($null,'ImTipTestTarget')
  if ($w -ne [IntPtr]::Zero) { break }
}
Write-Output ("target hwnd=0x{0:X}" -f [int64]$w)

$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 5

if ([C]::Caps() -ne 0) { [C]::Tap(0x14); Start-Sleep -Milliseconds 900 }
Write-Output ("caps baseline = " + [C]::Caps())

function Probe($label) {
  Start-Sleep -Milliseconds 1800       # 300ms 稳定 + 去抖 + 渲染; 不按键, 由前面的状态变更触发
  $line = Get-Content $log | Where-Object { $_ -match 'trigger key=' } | Select-Object -Last 1
  if ($line -match 'key=\([^,]+,\s*(\w+),\s*(\d+),\s*(\d+),\s*(\d+)\)') {
    $isCn=$Matches[1]; $sym=$Matches[2]; $lang=$Matches[3]; $caps=$Matches[4]
    $mode = if ($isCn -eq 'true') { '中文' } else { '英文' }
    $g1 = if ($isCn -eq 'true') { '中' } else { '英' }
    $g2 = if ($caps -eq '1' -and $isCn -eq 'false') { 'A' } else { if ($sym -eq '2') {'。'} else {'·'} }
    Write-Output ("{0,-12} 系统 caps={1} -> is_cn={2}({3}) symbol={4} caps_used={5}  ==> 显示 [{6}] [{7}]" -f $label,[C]::Caps(),$isCn,$mode,$sym,$caps,$g1,$g2)
  } else { Write-Output ("{0,-12} 无 trigger 行" -f $label) }
  # 断言: 只有"英文模式 + caps on"才允许出现 A
  $expectA = ($isCn -eq 'false' -and $caps -eq '1')
  $gotA    = ($g2 -eq 'A')
  if ($expectA -ne $gotA) {
    Write-Output ("    !! FAIL 期望A={0} 实际A={1}" -f $expectA, $gotA)
    $script:failed = $true
  } else {
    Write-Output ("    ok  (期望A={0} 实际A={1})" -f $expectA, $gotA)
  }
  Start-Sleep -Milliseconds 300
}

if ($w -ne [IntPtr]::Zero) { [C]::SetForegroundWindow($w) | Out-Null; Start-Sleep -Milliseconds 900 }

# 每个用例: 只改变一个变量(caps 或 中英), 让应用自己因状态变化触发提示
if ([C]::Caps() -ne 0) { [C]::Tap(0x14); Start-Sleep -Milliseconds 900 }
if ($w -ne [IntPtr]::Zero) { [C]::SetForegroundWindow($w) | Out-Null; Start-Sleep -Milliseconds 600 }

# 1) 中文 + caps off
[C]::Tap(0xA0); Start-Sleep -Milliseconds 1200
Probe 'case1'
# 2) 中文 + caps on  -> 仍应显示 全角, 不应出现 A
[C]::Tap(0x14)
Probe 'case2'
# 3) 英文 + caps off
[C]::Tap(0x14); Start-Sleep -Milliseconds 600      # 先关 caps
[C]::Tap(0xA0)                                      # Shift 切英文
Probe 'case3'
# 4) 英文 + caps on  -> 应显示 A
[C]::Tap(0x14)
Probe 'case4'
[C]::Tap(0x14); Start-Sleep -Milliseconds 600

Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
if ($tgt) { Stop-Process -Id $tgt.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 800
Write-Output ("RESULT=" + $(if($failed){"FAIL"}else{"PASS"}))
Write-Output ("final count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
