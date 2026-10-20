' ImTipLite 静默启动脚本
' 双击运行:后台启动输入法提示(无控制台窗口)
Set WshShell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
here = fso.GetParentFolderName(WScript.ScriptFullName)
script = here & "\imtip_lite.py"

' 优先使用已安装的 Python 的 pythonw.exe
Dim pyw
pyw = ""
On Error Resume Next
pyw = WshShell.RegRead("HKCU\Software\Python\PythonCore\3.12\InstallPath\")
If pyw <> "" Then pyw = pyw & "pythonw.exe"
If pyw = "" Or Not fso.FileExists(pyw) Then
    pyw = ""
    On Error Resume Next
    pyw = WshShell.RegRead("HKLM\SOFTWARE\Python\PythonCore\3.12\InstallPath\")
    If pyw <> "" Then pyw = pyw & "pythonw.exe"
End If

If pyw = "" Or Not fso.FileExists(pyw) Then
    ' 回退:尝试 PATH 中的 pythonw
    pyw = "pythonw.exe"
End If

WshShell.Run """" & pyw & """ """ & script & """", 0, False
