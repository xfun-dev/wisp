# Windows-side observer for scripts/e2e.py: reports what Windows really sees.
#   -Cmd cursor          physical cursor position as "x,y"
#   -Cmd key -Arg <vk>   "down" or "up" for a virtual-key code
#   -Cmd shown           "shown" or "hidden": whether Windows draws the pointer right now
#   -Cmd clipget         clipboard text, UTF-8 base64 (survives any console code page)
#   -Cmd clipset -Arg b  set clipboard text from UTF-8 base64
param([string]$Cmd, [string]$Arg = '')

Add-Type -Namespace E2E -Name User32 -MemberDefinition @'
public struct POINT { public int X; public int Y; }
[DllImport("user32.dll")] public static extern bool GetPhysicalCursorPos(out POINT p);
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(System.IntPtr ctx);
[StructLayout(LayoutKind.Sequential)] public struct CURSORINFO { public int cbSize; public int flags; public System.IntPtr hCursor; public POINT pt; }
[DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO ci);
'@
# physical pixels regardless of the display scale (PowerShell itself is DPI-unaware)
[E2E.User32]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null
# raw clipboard text: Get-Clipboard splits and rejoins lines, which hides CRLF vs LF
Add-Type -AssemblyName System.Windows.Forms

switch ($Cmd) {
    'cursor' {
        $p = New-Object E2E.User32+POINT
        [E2E.User32]::GetPhysicalCursorPos([ref]$p) | Out-Null
        "$($p.X),$($p.Y)"
    }
    'shown' {
        $ci = New-Object E2E.User32+CURSORINFO
        $ci.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($ci)
        [E2E.User32]::GetCursorInfo([ref]$ci) | Out-Null
        if (($ci.flags -band 1) -and $ci.hCursor -ne [IntPtr]::Zero) { 'shown' } else { 'hidden' }
    }
    'key' {
        if ([E2E.User32]::GetAsyncKeyState([int]$Arg) -band 0x8000) { 'down' } else { 'up' }
    }
    'clipget' {
        $t = [Windows.Forms.Clipboard]::GetText()
        [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($t))
    }
    'clipset' {
        [Windows.Forms.Clipboard]::SetText([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($Arg)))
        'ok'
    }
}
