// Throwaway spike S3: a private window station + desktop gives an isolated clipboard.
// Host mode (no args): creates station + desktop, starts itself as a child inside them.
// Child mode ("child" <outfile>): writes and reads text + image on the clipboard, writes a report.
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

static class S3 {
    const uint GENERIC_ALL = 0x10000000;
    const uint CF_UNICODETEXT = 13, CF_DIB = 8, GMEM_MOVEABLE = 2;
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO { public int cb; public string lpReserved, lpDesktop, lpTitle; public int x, y, xs, ys, xc, yc, fill, flags; public short show, res2; public IntPtr res2p, hin, hout, herr; }
    [StructLayout(LayoutKind.Sequential)]
    struct PI { public IntPtr hProcess, hThread; public int pid, tid; }
    [DllImport("user32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowStationW(string n, uint f, uint a, IntPtr sa);
    [DllImport("user32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateDesktopW(string n, IntPtr d, IntPtr dm, uint f, uint a, IntPtr sa);
    [DllImport("user32")] static extern bool CloseDesktop(IntPtr h);
    [DllImport("user32")] static extern bool CloseWindowStation(IntPtr h);
    [DllImport("user32")] static extern bool SetProcessWindowStation(IntPtr h);
    [DllImport("user32")] static extern uint GetClipboardSequenceNumber();
    [DllImport("user32", SetLastError = true)] static extern bool OpenClipboard(IntPtr h);
    [DllImport("user32")] static extern bool CloseClipboard();
    [DllImport("user32")] static extern bool EmptyClipboard();
    [DllImport("user32")] static extern IntPtr SetClipboardData(uint f, IntPtr h);
    [DllImport("user32")] static extern IntPtr GetClipboardData(uint f);
    [DllImport("user32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowExW(int ex, string cls, string name, int st, int x, int y, int w, int h, IntPtr parent, IntPtr m, IntPtr inst, IntPtr p);
    [DllImport("user32")] static extern IntPtr GetProcessWindowStation();
    [DllImport("user32")] static extern IntPtr GetThreadDesktop(int tid);
    [DllImport("user32", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool GetUserObjectInformationW(IntPtr h, int idx, StringBuilder sb, int len, out int need);
    [DllImport("kernel32")] static extern int GetCurrentThreadId();
    [DllImport("kernel32")] static extern IntPtr GlobalAlloc(uint f, UIntPtr n);
    [DllImport("kernel32")] static extern IntPtr GlobalLock(IntPtr h);
    [DllImport("kernel32")] static extern bool GlobalUnlock(IntPtr h);
    [DllImport("kernel32")] static extern UIntPtr GlobalSize(IntPtr h);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessW(string app, string cmd, IntPtr pa, IntPtr ta, bool inh, uint fl, IntPtr env, string dir, ref STARTUPINFO si, out PI pi);
    [DllImport("kernel32")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32")] static extern bool GetExitCodeProcess(IntPtr h, out uint c);

    static string Name(IntPtr h) { var sb = new StringBuilder(256); int n; GetUserObjectInformationW(h, 2, sb, 512, out n); return sb.ToString(); }

    static int Main(string[] a) {
        if (a.Length > 1 && a[0] == "child") return Child(a[1]);
        uint before = GetClipboardSequenceNumber();
        Console.WriteLine("host: user clipboard seq BEFORE = " + before + ", host station=" + Name(GetProcessWindowStation()));
        string ws = "S3Station_" + Process.GetCurrentProcess().Id;
        IntPtr hws = CreateWindowStationW(null, 0, 0x000F037F, IntPtr.Zero);
        if (hws == IntPtr.Zero) { Console.WriteLine("CreateWindowStation failed " + Marshal.GetLastWin32Error()); return 2; }
        ws = Name(hws); Console.WriteLine("host: created station " + ws);
        IntPtr orig = GetProcessWindowStation();
        SetProcessWindowStation(hws);
        IntPtr hd = CreateDesktopW("S3Desktop", IntPtr.Zero, IntPtr.Zero, 0, GENERIC_ALL, IntPtr.Zero);
        int err = Marshal.GetLastWin32Error();
        SetProcessWindowStation(orig);
        if (hd == IntPtr.Zero) { Console.WriteLine("CreateDesktop failed " + err); return 3; }
        var si = new STARTUPINFO(); si.cb = Marshal.SizeOf(si); si.lpDesktop = ws + "\\S3Desktop";
        PI pi;
        string exe = Process.GetCurrentProcess().MainModule.FileName;
        string outf = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "s3-child.txt");
        System.IO.File.Delete(outf);
        if (!CreateProcessW(exe, "\"" + exe + "\" child \"" + outf + "\"", IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, null, ref si, out pi)) { Console.WriteLine("CreateProcess failed " + Marshal.GetLastWin32Error()); return 4; }
        WaitForSingleObject(pi.hProcess, 20000);
        uint code; GetExitCodeProcess(pi.hProcess, out code);
        Console.WriteLine("child exit code = " + code);
        Console.WriteLine(System.IO.File.Exists(outf) ? System.IO.File.ReadAllText(outf) : "(no child output)");
        uint after = GetClipboardSequenceNumber();
        Console.WriteLine("host: user clipboard seq AFTER = " + after + (after == before ? "  UNCHANGED" : "  CHANGED"));
        CloseDesktop(hd); CloseWindowStation(hws);
        return (code == 0 && after == before) ? 0 : 1;
    }

    static int Child(string outFile) {
        var sw = new System.IO.StringWriter();
        int rc = Run(sw);
        System.IO.File.WriteAllText(outFile, sw.ToString());
        return rc;
    }

    static int Run(System.IO.StringWriter sw) {
        sw.WriteLine("child: station=" + Name(GetProcessWindowStation()) + " desktop=" + Name(GetThreadDesktop(GetCurrentThreadId())));
        sw.WriteLine("child: clipboard seq at start = " + GetClipboardSequenceNumber());
        IntPtr hwnd = CreateWindowExW(0, "STATIC", "s3", 0, 0, 0, 0, 0, new IntPtr(-3), IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
        if (hwnd == IntPtr.Zero) { sw.WriteLine("CreateWindow failed " + Marshal.GetLastWin32Error()); return 10; }
        string txt = "s3 secret text æøå";
        if (!OpenClipboard(hwnd)) { sw.WriteLine("OpenClipboard failed " + Marshal.GetLastWin32Error()); return 11; }
        EmptyClipboard();
        byte[] tb = Encoding.Unicode.GetBytes(txt + "\0");
        IntPtr ht = GlobalAlloc(GMEM_MOVEABLE, (UIntPtr)tb.Length); Marshal.Copy(tb, 0, GlobalLock(ht), tb.Length); GlobalUnlock(ht);
        SetClipboardData(CF_UNICODETEXT, ht);
        byte[] dib = new byte[40 + 16];
        BitConverter.GetBytes(40).CopyTo(dib, 0); BitConverter.GetBytes(2).CopyTo(dib, 4); BitConverter.GetBytes(2).CopyTo(dib, 8);
        BitConverter.GetBytes((short)1).CopyTo(dib, 12); BitConverter.GetBytes((short)24).CopyTo(dib, 14); BitConverter.GetBytes(16).CopyTo(dib, 20);
        for (int i = 40; i < dib.Length; i++) dib[i] = (byte)(i * 7);
        IntPtr hi = GlobalAlloc(GMEM_MOVEABLE, (UIntPtr)dib.Length); Marshal.Copy(dib, 0, GlobalLock(hi), dib.Length); GlobalUnlock(hi);
        SetClipboardData(CF_DIB, hi);
        CloseClipboard();
        sw.WriteLine("child: wrote text+image, seq now = " + GetClipboardSequenceNumber());
        if (!OpenClipboard(hwnd)) { sw.WriteLine("reopen failed"); return 12; }
        IntPtr rt = GetClipboardData(CF_UNICODETEXT); string back = Marshal.PtrToStringUni(GlobalLock(rt)); GlobalUnlock(rt);
        IntPtr ri = GetClipboardData(CF_DIB); int isz = (int)(ulong)GlobalSize(ri); byte[] rb = new byte[isz]; Marshal.Copy(GlobalLock(ri), rb, 0, isz); GlobalUnlock(ri);
        CloseClipboard();
        bool tok = back == txt, iok = isz >= dib.Length && rb[45] == dib[45] && rb[52] == dib[52];
        sw.WriteLine("child: text roundtrip " + (tok ? "OK" : "FAIL") + ", image roundtrip " + (iok ? "OK (" + isz + " bytes)" : "FAIL"));
        return tok && iok ? 0 : 13;
    }
}
