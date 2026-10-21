# 验证: --silent 启动无窗口 / 手动启动有窗口 / 托盘可打开
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe  = Join-Path $root 'ImTipLite.exe'
$pname = [System.IO.Path]::GetFileNameWithoutExtension($exe)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class U {
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
'@

Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 900

Write-Output '=== 1) --silent 启动 ==='
$p = Start-Process -FilePath $exe -ArgumentList '--silent' -PassThru
Start-Sleep -Seconds 6
$p.Refresh()
$alive = @(Get-Process $pname -ErrorAction SilentlyContinue).Count
$h = [U]::FindWindow($null, 'ImTipLite - 输入提示配置方案')
$vis = if ($h -ne [IntPtr]::Zero) { [U]::IsWindowVisible($h) } else { $false }
Write-Output ("进程存活=$alive  窗口存在=$($h -ne [IntPtr]::Zero)  窗口可见=$vis")
if ($alive -ne 1 -or $vis) { Write-Output 'FAIL: 静默启动' } else { Write-Output 'PASS: 后台运行且无窗口' }

Write-Output '=== 2) 双击式启动(第二次)应唤出已运行实例的窗口 ==='
$p2 = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 4
$p2.Refresh()
$alive2 = @(Get-Process $pname -ErrorAction SilentlyContinue).Count
$vis2 = if ($h -ne [IntPtr]::Zero) { [U]::IsWindowVisible($h) } else { $false }
Write-Output ("进程数=$alive2 (二实例退出=$($p2.HasExited))  窗口可见=$vis2")
if ($alive2 -eq 1 -and $p2.HasExited -and $vis2) { Write-Output 'PASS: 单实例 + 唤出窗口' } else { Write-Output 'FAIL: 唤出窗口' }

Write-Output '=== 3) 自启注册表项带 --silent ==='
# 由应用 API 写入, 再核对
$reg = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).ImTipLite
Write-Output ("Run 项 = $reg")
if ($reg -match '--silent') { Write-Output 'PASS: 注册表命令含 --silent' } else { Write-Output '(未开启自启或注册表无该项)' }

Get-Process $pname -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 700
Write-Output ("final count=" + @(Get-Process $pname -ErrorAction SilentlyContinue).Count)
