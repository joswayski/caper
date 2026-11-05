using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

public static class In
{
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll", EntryPoint = "SetCursorPos")] static extern bool RawSetCursorPos(int x, int y);
    public static bool SetCursorPos(int x, int y) { MoveAbs(x, y); return true; }
    static void Send(uint flags, uint data = 0) { INPUT i = new INPUT(); i.type = 0; i.mi.dwFlags = flags; i.mi.mouseData = data; SendInput(1, new INPUT[] { i }, Marshal.SizeOf(typeof(INPUT))); }
    static double Now() { return Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency; }
    // button: 0 left, 1 middle
    public static void Down(int b) { Send(b == 0 ? 0x0002u : 0x0020u); }
    public static void Up(int b) { Send(b == 0 ? 0x0004u : 0x0040u); }
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    public static void MoveAbs(int x, int y) { int w = GetSystemMetrics(0), h = GetSystemMetrics(1); INPUT i = new INPUT(); i.type = 0; i.mi.dx = (int)Math.Round(x * 65535.0 / (w - 1)); i.mi.dy = (int)Math.Round(y * 65535.0 / (h - 1)); i.mi.dwFlags = 0x8001; SendInput(1, new INPUT[] { i }, Marshal.SizeOf(typeof(INPUT))); }
    public static void MoveBy(int dx, int dy) { INPUT i = new INPUT(); i.type = 0; i.mi.dx = dx; i.mi.dy = dy; i.mi.dwFlags = 0x0001; SendInput(1, new INPUT[] { i }, Marshal.SizeOf(typeof(INPUT))); }
    public static void ClickAt(int x, int y) { SetCursorPos(x, y); Thread.Sleep(60); Down(0); Thread.Sleep(40); Up(0); }
    public static void Drag(int b, int x0, int y0, int x1, int y1, int steps)
    {
        SetCursorPos(x0, y0); Thread.Sleep(80); Down(b); Thread.Sleep(60);
        for (int s = 1; s <= steps; s++) { SetCursorPos(x0 + (x1 - x0) * s / steps, y0 + (y1 - y0) * s / steps); Thread.Sleep(8); }
        Thread.Sleep(60); Up(b);
    }
    // Press at (x,y), then move back and forth by +-dx/dy every ~8 ms for `seconds`, then release.
    public static void DragFor(int b, int x, int y, int dx, int dy, double seconds)
    {
        SetCursorPos(x, y); Thread.Sleep(60); Down(b); Thread.Sleep(40);
        double end = Now() + seconds; double t0 = Now();
        while (Now() < end)
        {
            double ph = Math.Sin((Now() - t0) * 2 * Math.PI / 1.5);
            SetCursorPos(x + (int)(dx * ph), y + (int)(dy * ph));
            Thread.Sleep(8);
        }
        Up(b);
    }
    // Wheel at (x,y), alternating direction every `flip` notches.
    public static void WheelFor(int x, int y, double seconds, int flip)
    {
        SetCursorPos(x, y); Thread.Sleep(60);
        double end = Now() + seconds; int n = 0;
        while (Now() < end) { int dir = ((n / flip) % 2 == 0) ? 120 : -120; Send(0x0800, (uint)dir); n++; Thread.Sleep(50); }
    }
    // Sweep the cursor over a rectangle in rows for `seconds`.
    public static void SweepFor(int x0, int y0, int x1, int y1, int step, double seconds)
    {
        double end = Now() + seconds; bool down = true;
        while (Now() < end)
        {
            for (int y = (down ? y0 : y1); (down ? y <= y1 : y >= y0) && Now() < end; y += (down ? step : -step))
            {
                int x = x0 + (int)(((y - y0) * 3) % Math.Max(1, (x1 - x0)));
                SetCursorPos(x, y); Thread.Sleep(30);
            }
            down = !down;
        }
    }
}


