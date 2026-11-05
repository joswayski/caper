//! SCRATCH in-process sampling profiler for Windows (no admin): a helper
//! thread suspends the UI thread every ~1 ms, unwinds its stack with
//! RtlVirtualUnwind and records the frames. Enabled by SAMPLE_OUT=<path>;
//! only samples while `set_active(true)`. `dump()` symbolizes via dbghelp
//! (needs the .pdb) and writes self/inclusive tables.
#![allow(clippy::all)]
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Diagnostics::Debug::*;
use windows_sys::Win32::System::Threading::*;

const DEPTH: usize = 48;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static STACKS: Mutex<Vec<[u64; DEPTH]>> = Mutex::new(Vec::new());
static STARTED: AtomicBool = AtomicBool::new(false);
static CTX_FAIL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_ERR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(p: u32) -> u32;
}

pub fn start() {
    if std::env::var_os("SAMPLE_OUT").is_none() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let tid = unsafe { GetCurrentThreadId() };
    let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, tid) } as usize;
    STACKS.lock().unwrap().reserve(64_000);
    std::thread::spawn(move || unsafe {
        timeBeginPeriod(1);
        let h = handle as HANDLE;
        let mut buf: Vec<[u64; DEPTH]> = Vec::with_capacity(64_000);
        while !STOP.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(1));
            if !ACTIVE.load(Ordering::Relaxed) || buf.len() == buf.capacity() {
                continue;
            }
            let mut frames = [0u64; DEPTH];
            if SuspendThread(h) == u32::MAX {
                continue;
            }
            #[repr(C, align(16))]
            struct Aligned(CONTEXT);
            let mut aligned: Aligned = std::mem::zeroed();
            let ctx = &mut aligned.0;
            ctx.ContextFlags = CONTEXT_FULL_AMD64;
            if GetThreadContext(h, ctx) != 0 {
                for f in frames.iter_mut() {
                    let pc = ctx.Rip;
                    if pc == 0 {
                        break;
                    }
                    *f = pc;
                    let mut base = 0u64;
                    let entry = RtlLookupFunctionEntry(pc, &mut base, std::ptr::null_mut());
                    if entry.is_null() {
                        // Leaf function: return address is at [rsp].
                        ctx.Rip = *(ctx.Rsp as *const u64);
                        ctx.Rsp += 8;
                    } else {
                        let mut data = std::ptr::null_mut();
                        let mut frame = 0u64;
                        RtlVirtualUnwind(UNW_FLAG_NHANDLER, base, pc, entry, ctx, &mut data, &mut frame, std::ptr::null_mut());
                    }
                }
            } else {
                CTX_FAIL.fetch_add(1, Ordering::Relaxed);
                LAST_ERR.store(windows_sys::Win32::Foundation::GetLastError(), Ordering::Relaxed);
            }
            ResumeThread(h);
            buf.push(frames);
        }
        STACKS.lock().unwrap().extend(buf);
        CloseHandle(h);
    });
}

pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

pub fn dump() {
    let Ok(path) = std::env::var("SAMPLE_OUT") else { return };
    ACTIVE.store(false, Ordering::Relaxed);
    STOP.store(true, Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let stacks = std::mem::take(&mut *STACKS.lock().unwrap());
    unsafe {
        let proc_ = GetCurrentProcess();
        SymSetOptions(SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS);
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap();
        let search = format!("{};{}\\deps\0", dir.display(), dir.display());
        let ok = SymInitialize(proc_, search.as_ptr(), 1);
        if ok == 0 { eprintln!("SymInitialize failed {}", windows_sys::Win32::Foundation::GetLastError()); }
        let mut names: HashMap<u64, String> = HashMap::new();
        let mut name = |addr: u64| -> String {
            names
                .entry(addr)
                .or_insert_with(|| {
                    #[repr(C, align(8))]
                    struct Buf([u8; std::mem::size_of::<SYMBOL_INFO>() + 1024]);
                    let mut b = Buf([0; std::mem::size_of::<SYMBOL_INFO>() + 1024]);
                    let sym = b.0.as_mut_ptr() as *mut SYMBOL_INFO;
                    (*sym).SizeOfStruct = std::mem::size_of::<SYMBOL_INFO>() as u32;
                    (*sym).MaxNameLen = 1024;
                    let mut disp = 0u64;
                    let module = {
                        use windows_sys::Win32::System::LibraryLoader::*;
                        let mut hm = std::ptr::null_mut();
                        let mut buf = [0u16; 512];
                        if GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, addr as *const u16, &mut hm) != 0 {
                            let n = GetModuleFileNameW(hm, buf.as_mut_ptr(), 512) as usize;
                            let s = String::from_utf16_lossy(&buf[..n]);
                            s.rsplit('\\').next().unwrap_or("").to_lowercase()
                        } else { "?".into() }
                    };
                    if SymFromAddr(proc_, addr, &mut disp, sym) != 0 {
                        let n = std::slice::from_raw_parts((*sym).Name.as_ptr() as *const u8, (*sym).NameLen as usize);
                        format!("{module}!{}", String::from_utf8_lossy(n))
                    } else {
                        format!("{module}!?{addr:x}")
                    }
                })
                .clone()
        };
        let mut selfc: HashMap<String, usize> = HashMap::new();
        let mut incl: HashMap<String, usize> = HashMap::new();
        let mut folded: HashMap<String, usize> = HashMap::new();
        for s in &stacks {
            let frames: Vec<String> = s.iter().take_while(|a| **a != 0).enumerate().map(|(i, a)| name(if i == 0 { *a } else { *a - 1 })).collect();
            if let Some(f) = frames.first() {
                *selfc.entry(f.clone()).or_default() += 1;
            }
            let mut seen = std::collections::HashSet::new();
            for f in &frames {
                if seen.insert(f.clone()) {
                    *incl.entry(f.clone()).or_default() += 1;
                }
            }
            let mut rev = frames.clone();
            rev.reverse();
            *folded.entry(rev.join(";")).or_default() += 1;
        }
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let total = stacks.len().max(1);
        let _ = writeln!(f, "samples {} ctx_fail {} last_err {}", stacks.len(), CTX_FAIL.load(Ordering::Relaxed), LAST_ERR.load(Ordering::Relaxed));
        let mut v: Vec<_> = selfc.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let _ = writeln!(f, "## self");
        for (n, c) in v.iter().take(80) {
            let _ = writeln!(f, "{:6.2}% {c:6} {n}", *c as f64 * 100.0 / total as f64);
        }
        let mut v: Vec<_> = incl.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let _ = writeln!(f, "## inclusive");
        for (n, c) in v.iter().take(250) {
            let _ = writeln!(f, "{:6.2}% {c:6} {n}", *c as f64 * 100.0 / total as f64);
        }
        let mut ff = std::io::BufWriter::new(std::fs::File::create(format!("{path}.folded")).unwrap());
        for (k, c) in folded {
            let _ = writeln!(ff, "{k} {c}");
        }
    }
}
