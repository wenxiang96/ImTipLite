# Minimal WinForms text box used as an input target for the ImTipLite runtime check.
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$f = New-Object System.Windows.Forms.Form
$f.Text = 'ImTipTestTarget'
$f.Width = 520
$f.Height = 220
$t = New-Object System.Windows.Forms.TextBox
$t.Multiline = $true
$t.Dock = 'Fill'
$t.Font = New-Object System.Drawing.Font('Consolas', 14)
$f.Controls.Add($t)
$f.Add_Shown({ $t.Focus(); $f.Activate() })
[System.Windows.Forms.Application]::Run($f)
