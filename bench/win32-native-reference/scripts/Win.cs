using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;

public static class W
{
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern int GetWindowTextLength(IntPtr h);

    // Precise CPU time: Windows charges GetProcessTimes at clock ticks
    // (15.6 ms), which misses short bursts; cycle counters are exact (TSC).
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint access, bool inherit, int tid);
    [DllImport("kernel32.dll")] static extern bool QueryProcessCycleTime(IntPtr h, out ulong cycles);
    [DllImport("kernel32.dll")] static extern bool QueryThreadCycleTime(IntPtr h, out ulong cycles);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentThread();
    public static double TscHz;
    public static double CalibrateTsc()
    {
        double best = 0;
        for (int k = 0; k < 3; k++)
        {
            ulong c0, c1; QueryThreadCycleTime(GetCurrentThread(), out c0);
            var sw = Stopwatch.StartNew();
            while (sw.Elapsed.TotalMilliseconds < 300) { }
            QueryThreadCycleTime(GetCurrentThread(), out c1);
            best = Math.Max(best, (c1 - c0) / sw.Elapsed.TotalSeconds);
        }
        TscHz = best; return best;
    }
    public static double ProcessCpuSeconds(int pid)
    {
        IntPtr h = OpenProcess(0x1000, false, pid); ulong c; QueryProcessCycleTime(h, out c); return c / TscHz;
    }
    public static double ThreadCpuSeconds(int tid)
    {
        IntPtr h = OpenThread(0x0800, false, tid); ulong c; QueryThreadCycleTime(h, out c); return c / TscHz;
    }

    public static IntPtr FindMain(int pid)
    {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) =>
        {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h))
            {
                RECT r;
                GetClientRect(h, out r);
                if (r.R > 400 && r.B > 300) { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static int UiThread(IntPtr h) { uint p; return (int)GetWindowThreadProcessId(h, out p); }

    // Client rect in screen pixels: x, y, w, h.
    public static int[] Client(IntPtr h)
    {
        RECT r; GetClientRect(h, out r);
        POINT p = new POINT(); ClientToScreen(h, ref p);
        return new int[] { p.X, p.Y, r.R, r.B };
    }

    // Raise above everything (topmost) at a fixed position, optionally resizing.
    public static void Place(IntPtr h, int x, int y, int w, int hh)
    {
        uint flags = 0x0040; // SHOWWINDOW
        if (w <= 0) flags |= 0x0001; // NOSIZE
        SetWindowPos(h, new IntPtr(-1), x, y, w, hh, flags);
        keybd_event(0x12, 0, 0, UIntPtr.Zero); // Alt lets a background process take the foreground.
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        SetForegroundWindow(h);
    }

    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; [FieldOffset(8)] public KEYBDINPUT ki; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);

    static void Send(INPUT i) { SendInput(1, new INPUT[] { i }, Marshal.SizeOf(typeof(INPUT))); }
    public static void Wheel(int delta) { INPUT i = new INPUT(); i.type = 0; i.mi.mouseData = (uint)delta; i.mi.dwFlags = 0x0800; Send(i); }
    public static void Click() { INPUT i = new INPUT(); i.type = 0; i.mi.dwFlags = 0x0002; Send(i); i.mi.dwFlags = 0x0004; Send(i); }
    public static void Key(ushort vk)
    {
        INPUT i = new INPUT(); i.type = 1; i.ki.vk = vk; Send(i);
        i.ki.flags = 0x0002; Send(i);
    }
    public static void ClearField()
    {
        INPUT i = new INPUT(); i.type = 1;
        i.ki.vk = 0x11; Send(i); i.ki.vk = 0x41; Send(i);
        i.ki.flags = 2; Send(i); i.ki.vk = 0x11; Send(i);
        i.ki.flags = 0; Key(0x08);
    }
    public static void TypeString(string s) { foreach (char c in s) { TypeChar(c); Thread.Sleep(15); } }
    public static void TypeChar(char c)
    {
        INPUT i = new INPUT(); i.type = 1; i.ki.scan = c; i.ki.flags = 0x0004; Send(i);
        i.ki.flags = 0x0004 | 0x0002; Send(i);
    }

    static double Now() { return Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency; }

    // Sweeps the pointer up and down a region of the client area (fractions),
    // 20 logical px per step every 30 ms, like the Linux driver.
    public static void Hover(IntPtr h, double seconds)
    {
        int[] c = Client(h);
        double scale = GetDpiForWindow(h) / 96.0;
        int x0 = c[0] + (int)(c[2] * 0.30), y0 = c[1] + (int)(c[3] * 0.12), y1 = c[1] + (int)(c[3] * 0.80);
        int step = (int)(20 * scale);
        double end = Now() + seconds;
        while (Now() < end)
        {
            for (int y = y0; y <= y1 && Now() < end; y += step) { SetCursorPos(x0 + (int)(((y - y0) / scale) % 200 * scale), y); Thread.Sleep(30); }
            for (int y = y1; y >= y0 && Now() < end; y -= step) { SetCursorPos(x0 + (int)(((y - y0) / scale) % 200 * scale), y); Thread.Sleep(30); }
        }
    }

    public static void WheelFor(int delta, double seconds)
    {
        double end = Now() + seconds;
        while (Now() < end) { Wheel(delta); Thread.Sleep(30); }
    }

    // Live messages every 100 ms through the fixture's control endpoint.
    public static int BurstFor(string url, double seconds)
    {
        double end = Now() + seconds; int i = 0;
        var client = new System.Net.WebClient();
        while (Now() < end)
        {
            i++;
            double next = Now() + 0.1;
            client.Headers[System.Net.HttpRequestHeader.ContentType] = "application/json";
            client.Encoding = System.Text.Encoding.UTF8;
            client.UploadString(url, "{\"incomingMessage\":{\"channelId\":\"chan00000001\",\"text\":\"live message " + i + " with a few more words so it wraps like real chat does \\ud83c\\udf89\"}}");
            int wait = (int)((next - Now()) * 1000); if (wait > 0) Thread.Sleep(wait);
        }
        return i;
    }

    // Window width sweeps 1000-1500 logical px (sine, period 2 s) at ~60 Hz.
    public static void ResizeFor(IntPtr h, int x, int y, int heightPx, double seconds)
    {
        double scale = GetDpiForWindow(h) / 96.0;
        double start = Now();
        while (Now() - start < seconds)
        {
            double phase = Math.Sin((Now() - start) * Math.PI);
            int w = (int)((1250 + 250 * phase) * scale);
            SetWindowPos(h, new IntPtr(-1), x, y, w, heightPx, 0x0010); // NOACTIVATE
            Thread.Sleep(16);
        }
    }

    public static void Screenshot(IntPtr h, string path)
    {
        int[] c = Client(h);
        using (var bmp = new System.Drawing.Bitmap(c[2], c[3]))
        using (var g = System.Drawing.Graphics.FromImage(bmp))
        {
            g.CopyFromScreen(c[0], c[1], 0, 0, new System.Drawing.Size(c[2], c[3]));
            bmp.Save(path, System.Drawing.Imaging.ImageFormat.Png);
        }
    }

    public static void TypeFor(string text, double seconds)
    {
        double end = Now() + seconds; int k = 0;
        while (Now() < end) { TypeChar(text[k++ % text.Length]); Thread.Sleep(50); }
    }
}

// Samples \GPU Engine(pid_N_*)\Utilization Percentage and
// \GPU Process Memory(pid_N_*)\Dedicated Usage once per second on a thread.
public class GpuSampler
{
    Thread thread; volatile bool stop; int pid;
    public List<Dictionary<string, double>> Samples = new List<Dictionary<string, double>>();
    public string Error = "";

    public GpuSampler(int pid) { this.pid = pid; }

    public void Start() { thread = new Thread(Run); thread.IsBackground = true; thread.Start(); }
    public void Stop() { stop = true; if (thread != null) thread.Join(5000); }

    void Run()
    {
        try
        {
            string prefix = "pid_" + pid + "_";
            var engines = new List<KeyValuePair<string, PerformanceCounter>>();
            var mems = new List<PerformanceCounter>();
            DateTime deadline = DateTime.Now.AddSeconds(5);
            while (engines.Count == 0 && !stop && DateTime.Now < deadline)
            {
                foreach (var n in new PerformanceCounterCategory("GPU Engine").GetInstanceNames().Where(n => n.StartsWith(prefix)))
                {
                    int e = n.IndexOf("engtype_");
                    string type = e >= 0 ? n.Substring(e + 8) : "other";
                    engines.Add(new KeyValuePair<string, PerformanceCounter>(type, new PerformanceCounter("GPU Engine", "Utilization Percentage", n, true)));
                }
                if (engines.Count == 0) Thread.Sleep(200);
            }
            foreach (var n in new PerformanceCounterCategory("GPU Process Memory").GetInstanceNames().Where(n => n.StartsWith(prefix)))
                mems.Add(new PerformanceCounter("GPU Process Memory", "Dedicated Usage", n, true));
            foreach (var p in engines) p.Value.NextValue();
            while (!stop)
            {
                Thread.Sleep(1000);
                if (stop) break;
                var s = new Dictionary<string, double>();
                foreach (var p in engines)
                {
                    double v = 0;
                    try { v = p.Value.NextValue(); } catch { }
                    double old; s.TryGetValue(p.Key, out old); s[p.Key] = old + v;
                }
                double mem = 0;
                foreach (var m in mems) { try { mem += m.NextValue(); } catch { } }
                s["dedicated_mib"] = mem / 1048576.0;
                Samples.Add(s);
            }
        }
        catch (Exception e) { Error = e.Message; }
    }

    // Mean utilization per engine type (percent of one engine), max dedicated memory.
    public string Json()
    {
        if (Samples.Count == 0) return "{\"samples\":0,\"error\":\"" + Error.Replace("\"", "'") + "\"}";
        var keys = Samples.SelectMany(s => s.Keys).Distinct().OrderBy(k => k);
        var parts = new List<string>();
        parts.Add("\"samples\":" + Samples.Count);
        foreach (var k in keys)
        {
            var vals = Samples.Select(s => { double v; return s.TryGetValue(k, out v) ? v : 0; }).ToList();
            double agg = k == "dedicated_mib" ? vals.Max() : vals.Average();
            parts.Add("\"" + (k == "dedicated_mib" ? k : "util_" + k) + "\":" + agg.ToString("0.00", System.Globalization.CultureInfo.InvariantCulture));
        }
        return "{" + string.Join(",", parts) + "}";
    }
}
