# Installs wisp.new.exe as the current wisp client, registers it to start at logon, and starts it.
# -Server pins the Mac's address; without it the client finds the Mac on the LAN.
param([string]$Server = '')

$dir = "$env:LOCALAPPDATA\Wisp"
$exe = "$dir\wisp.exe"
$wispArgs = @('client')
if ($Server) { $wispArgs += @('--server', $Server) }

Get-Process wisp -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 1
Move-Item -Force "$dir\wisp.new.exe" $exe
Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Wisp' -Value "`"$exe`" $($wispArgs -join ' ')"
Start-Process -FilePath $exe -ArgumentList $wispArgs
# Start menu entry for the status window
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Wisp.lnk'))
$shortcut.TargetPath = $exe
$shortcut.Arguments = 'status'
$shortcut.Save()
Start-Sleep 3
"RUN = " + (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run').Wisp
Get-Process wisp | Select Id,SessionId,@{n='PrivateMB';e={[math]::Round($_.PrivateMemorySize64/1MB,2)}} | Format-List
Get-Content -Encoding UTF8 "$dir\wisp.log"
