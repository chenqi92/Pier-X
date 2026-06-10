# Click at a window-relative pixel (matches the PrintWindow bitmap coords,
# which are 1:1 with the window's physical rect). Used to drive the GPUI app
# for screenshots.
param(
    [string]$ProcName = "pier-ui-gpui",
    [int]$X = 0,
    [int]$Y = 0
)

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public class Clk {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
    public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public const uint LDOWN = 0x0002, LUP = 0x0004;

    public static IntPtr Find(uint targetPid) {
        IntPtr best = IntPtr.Zero; int bestArea = 0;
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h)) return true;
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            RECT r; GetWindowRect(h, out r);
            int area = (r.Right - r.Left) * (r.Bottom - r.Top);
            if (area > bestArea) { bestArea = area; best = h; }
            return true;
        }, IntPtr.Zero);
        return best;
    }
}
"@

$p = Get-Process -Name $ProcName -ErrorAction Stop | Select-Object -First 1
$h = [Clk]::Find([uint32]$p.Id)
[Clk]::ShowWindow($h, 9) | Out-Null   # SW_RESTORE (in case minimized off-screen)
[Clk]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300
$r = New-Object Clk+RECT
[Clk]::GetWindowRect($h, [ref]$r) | Out-Null
$sx = $r.Left + $X; $sy = $r.Top + $Y
[Clk]::SetCursorPos($sx, $sy) | Out-Null
Start-Sleep -Milliseconds 150
[Clk]::mouse_event([Clk]::LDOWN, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 60
[Clk]::mouse_event([Clk]::LUP, 0, 0, 0, [IntPtr]::Zero)
Write-Output "clicked $sx,$sy"
