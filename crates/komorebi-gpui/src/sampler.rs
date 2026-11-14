//! An in-process sampling profiler for the A/B harness (feature `sampler`,
//! off in every shipped build; `scripts/opt-prof.ps1`, docs/PERF.md section
//! 8). Event Tracing for Windows' CPU sampling needs an administrator and the
//! Windows Performance Toolkit, so the app profiles itself.
//!
//! A thread wakes every millisecond and, for each thread of the process that
//! used CPU since the last look (`QueryThreadCycleTime` moved), suspends it,
//! reads its registers and walks its stack with the unwind tables
//! (`RtlLookupFunctionEntry`, `RtlVirtualUnwind`), then resumes it. While a
//! thread is suspended the sampler allocates nothing and prints nothing (the
//! suspended thread may hold the heap or the stderr lock): the samples go
//! into a buffer reserved before the first one. At the end the instruction
//! pointers are turned into function names, files and lines with dbghelp and
//! the exe's PDB (build with `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`
//! and run the exe from its build folder, so that `komorebi.pdb` is beside
//! it), and a report of the hottest functions (self time, with the file and
//! line of their hottest instruction, and inclusive time) is written.
//!
//! Started by `KOMOREBI_PROF_START=<name>` (at the top of `main`) or the
//! bench step `sample:<name>`, ended by the bench step `sample-end`; the
//! report goes to `KOMOREBI_PROF_DIR` (default: the current folder) as
//! `<name>.txt`.

use std::collections::HashMap;

/// Per (file, line) sample counts of one function.
type LineCounts = HashMap<(String, u32), usize>;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_FULL_AMD64, GetThreadContext, IMAGEHLP_LINE64, IMAGEHLP_MODULE64,
    RtlLookupFunctionEntry, RtlVirtualUnwind, SYMBOL_INFO, SYMOPT_DEFERRED_LOADS,
    SYMOPT_FAIL_CRITICAL_ERRORS, SYMOPT_LOAD_LINES, SYMOPT_NO_PROMPTS, SYMOPT_UNDNAME, SymFromAddr,
    SymGetLineFromAddr64, SymGetModuleInfo64, SymInitialize, SymSetOptions, UNW_FLAG_NHANDLER,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetThreadDescription, OpenThread,
    ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
    THREAD_SUSPEND_RESUME,
};
use windows::Win32::System::WindowsProgramming::QueryThreadCycleTime;

/// Frames kept per sample.
const DEPTH: usize = 48;
/// Samples kept (about 400 s of one busy thread at 1 kHz).
const CAP: usize = 400_000;
const PERIOD: Duration = Duration::from_micros(1000);

struct Sample {
    tid: u32,
    n: u8,
    ips: [u64; DEPTH],
}

#[repr(align(16))]
struct Aligned(CONTEXT);

static RUN: AtomicBool = AtomicBool::new(false);
static ACTIVE: Mutex<Option<(String, JoinHandle<Vec<Sample>>)>> = Mutex::new(None);

/// Starts sampling under `name` (a second start while one runs is ignored).
pub fn start(name: &str) {
    let Ok(mut active) = ACTIVE.lock() else {
        return;
    };
    if active.is_some() {
        return;
    }
    RUN.store(true, Ordering::SeqCst);
    let handle = std::thread::Builder::new()
        .name("opt-sampler".into())
        .spawn(sampler_loop);
    if let Ok(handle) = handle {
        *active = Some((name.to_owned(), handle));
    }
}

/// Stops the sampler and writes the report; returns the line to print.
pub fn stop_and_report() -> Option<String> {
    let (name, handle) = ACTIVE.lock().ok()?.take()?;
    RUN.store(false, Ordering::SeqCst);
    let samples = handle.join().ok()?;
    let dir = std::env::var_os("KOMOREBI_PROF_DIR").unwrap_or_else(|| ".".into());
    let dir = Path::new(&dir);
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{name}.txt"));
    let (text, table) = report(&name, &samples);
    let _ = std::fs::write(dir.join(format!("{name}.tsv")), table);
    match std::fs::write(&path, text) {
        Ok(()) => Some(format!(
            "[prof] {name} samples={} report={}",
            samples.len(),
            path.display()
        )),
        Err(e) => Some(format!("[prof] {name}: writing {}: {e}", path.display())),
    }
}

fn sampler_loop() -> Vec<Sample> {
    // Everything that allocates happens before the first suspension, or
    // while no thread is suspended.
    let mut out: Vec<Sample> = Vec::with_capacity(CAP);
    // tid -> (handle, cycles at the last look)
    let mut threads: HashMap<u32, (HANDLE, u64)> = HashMap::with_capacity(512);
    let me = unsafe { GetCurrentThreadId() };
    let mut round = 0u32;
    while RUN.load(Ordering::Relaxed) {
        if round.is_multiple_of(25) {
            refresh(&mut threads, me);
        }
        round = round.wrapping_add(1);
        for (&tid, (handle, last)) in &mut threads {
            let mut cycles = 0u64;
            // SAFETY: `handle` is an open thread handle with query rights.
            if unsafe { QueryThreadCycleTime(*handle, &mut cycles) }.is_err() || cycles == *last {
                continue;
            }
            *last = cycles;
            let mut sample = Sample {
                tid,
                n: 0,
                ips: [0; DEPTH],
            };
            // SAFETY: the thread is suspended while its registers and stack
            // are read, and resumed on every path; nothing between the two
            // allocates or locks.
            unsafe {
                if SuspendThread(*handle) == u32::MAX {
                    continue;
                }
                let mut ctx = Aligned(std::mem::zeroed());
                ctx.0.ContextFlags = CONTEXT_FULL_AMD64;
                if GetThreadContext(*handle, &mut ctx.0).is_ok() {
                    walk(&mut ctx.0, &mut sample);
                }
                ResumeThread(*handle);
            }
            if sample.n > 0 && out.len() < CAP {
                out.push(sample);
            }
        }
        std::thread::sleep(PERIOD); // timer-exempt: the sampling period of a development profiler
    }
    for (handle, _) in threads.into_values() {
        // SAFETY: handles this thread opened.
        let _ = unsafe { CloseHandle(handle) };
    }
    out
}

/// Opens the threads that appeared since the last look.
fn refresh(threads: &mut HashMap<u32, (HANDLE, u64)>, me: u32) {
    // SAFETY: a thread snapshot of this process, read to its end and closed.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return;
        };
        let pid = GetCurrentProcessId();
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut more = Thread32First(snap, &mut entry).is_ok();
        while more {
            if entry.th32OwnerProcessID == pid
                && entry.th32ThreadID != me
                && !threads.contains_key(&entry.th32ThreadID)
                && let Ok(h) = OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                    false,
                    entry.th32ThreadID,
                )
            {
                threads.insert(entry.th32ThreadID, (h, 0));
            }
            more = Thread32Next(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
}

/// The stack of a suspended thread, from its registers.
///
/// # Safety
/// `ctx` holds the registers of a thread that is suspended, so its stack is
/// not changing.
unsafe fn walk(ctx: &mut CONTEXT, sample: &mut Sample) {
    for i in 0..DEPTH {
        let rip = ctx.Rip;
        if rip == 0 {
            break;
        }
        sample.ips[i] = rip;
        sample.n += 1;
        // A caller's return address can be the first byte of the next
        // function when the call was the last instruction of the caller.
        let lookup = if i == 0 { rip } else { rip - 1 };
        let mut base = 0u64;
        // SAFETY: plain lookups in the loaded images' unwind tables.
        let entry = unsafe { RtlLookupFunctionEntry(lookup, &mut base, None) };
        if entry.is_null() {
            // A leaf function without unwind data: the return address is on
            // top of the stack.
            if ctx.Rsp == 0 || !ctx.Rsp.is_multiple_of(8) {
                break;
            }
            // SAFETY: the stack of a suspended thread, at its stack pointer.
            ctx.Rip = unsafe { *(ctx.Rsp as *const u64) };
            ctx.Rsp += 8;
        } else {
            let mut handler_data = std::ptr::null_mut();
            let mut establisher = 0u64;
            // SAFETY: `entry` is from the table of the image at `base`, and
            // `ctx` is the thread's own register set.
            unsafe {
                RtlVirtualUnwind(
                    UNW_FLAG_NHANDLER,
                    base,
                    lookup,
                    entry,
                    ctx,
                    &mut handler_data,
                    &mut establisher,
                    None,
                );
            }
        }
    }
}

// ------------------------------------------------------------- the report

#[derive(Clone, Default)]
struct Resolved {
    function: String,
    file: String,
    line: u32,
    module: String,
}

struct Symbols {
    cache: HashMap<u64, Resolved>,
}

impl Symbols {
    fn new() -> Self {
        // SAFETY: dbghelp is initialised once, for this process, from the
        // reporting thread; nothing else calls it.
        unsafe {
            SymSetOptions(
                SYMOPT_UNDNAME
                    | SYMOPT_LOAD_LINES
                    | SYMOPT_DEFERRED_LOADS
                    | SYMOPT_FAIL_CRITICAL_ERRORS
                    | SYMOPT_NO_PROMPTS,
            );
            let _ = SymInitialize(GetCurrentProcess(), windows::core::PCSTR::null(), true);
        }
        Self {
            cache: HashMap::new(),
        }
    }

    /// `ip` is a leaf instruction pointer or a return address (`caller`).
    fn resolve(&mut self, ip: u64, caller: bool) -> Resolved {
        let at = if caller { ip - 1 } else { ip };
        if let Some(r) = self.cache.get(&at) {
            return r.clone();
        }
        let mut r = Resolved::default();
        // SAFETY: dbghelp calls with correctly sized, initialised structures.
        unsafe {
            let process = GetCurrentProcess();
            let mut buf = vec![0u8; std::mem::size_of::<SYMBOL_INFO>() + 1024];
            let info = buf.as_mut_ptr().cast::<SYMBOL_INFO>();
            (*info).SizeOfStruct = std::mem::size_of::<SYMBOL_INFO>() as u32;
            (*info).MaxNameLen = 1024;
            let mut displacement = 0u64;
            if SymFromAddr(process, at, Some(&mut displacement), info).is_ok() {
                let name = std::slice::from_raw_parts(
                    (*info).Name.as_ptr().cast::<u8>(),
                    (*info).NameLen as usize,
                );
                r.function = String::from_utf8_lossy(name).into_owned();
            }
            let mut line = IMAGEHLP_LINE64 {
                SizeOfStruct: std::mem::size_of::<IMAGEHLP_LINE64>() as u32,
                ..Default::default()
            };
            let mut disp = 0u32;
            if SymGetLineFromAddr64(process, at, &mut disp, &mut line).is_ok()
                && !line.FileName.is_null()
            {
                r.file = line.FileName.to_string().unwrap_or_default();
                r.line = line.LineNumber;
            }
            let mut module = IMAGEHLP_MODULE64 {
                SizeOfStruct: std::mem::size_of::<IMAGEHLP_MODULE64>() as u32,
                ..Default::default()
            };
            if SymGetModuleInfo64(process, at, &mut module).is_ok() {
                let name: Vec<u8> = module
                    .ModuleName
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| c as u8)
                    .collect();
                r.module = String::from_utf8_lossy(&name).into_owned();
            }
        }
        if r.function.is_empty() {
            r.function = format!(
                "{}+{at:#x}",
                if r.module.is_empty() { "?" } else { &r.module }
            );
        }
        self.cache.insert(at, r.clone());
        r
    }
}

fn is_wait(function: &str) -> bool {
    [
        "Wait",
        "Sleep",
        "Delay",
        "GetMessage",
        "RemoveIoCompletion",
        "Park",
        "Idle",
        // The text-scale watcher thread blocks here for the whole run.
        "NotifyChangeKey",
    ]
    .iter()
    .any(|w| function.contains(w))
}

fn thread_name(tid: u32) -> String {
    // SAFETY: a short-lived handle on a thread of this process.
    unsafe {
        let Ok(h) = OpenThread(THREAD_QUERY_INFORMATION, false, tid) else {
            return String::new();
        };
        let name = GetThreadDescription(h)
            .ok()
            .and_then(|p| p.to_string().ok())
            .unwrap_or_default();
        let _ = CloseHandle(h);
        name
    }
}

/// The text report, and a tab-separated table of every function seen
/// (`function, self, inclusive, file, line, module`) for merging runs.
fn report(name: &str, samples: &[Sample]) -> (String, String) {
    let mut sym = Symbols::new();
    // function -> (count, per-line counts)
    let mut selfs: HashMap<String, (usize, LineCounts)> = HashMap::new();
    let mut incl: HashMap<String, usize> = HashMap::new();
    let mut waiting = 0usize;
    let mut by_thread: HashMap<u32, usize> = HashMap::new();
    let mut fn_module: HashMap<String, String> = HashMap::new();
    let mut modules: HashMap<String, usize> = HashMap::new();
    for s in samples {
        let leaf = sym.resolve(s.ips[0], false);
        *by_thread.entry(s.tid).or_default() += 1;
        if is_wait(&leaf.function) {
            waiting += 1;
            continue;
        }
        *modules.entry(leaf.module.clone()).or_default() += 1;
        fn_module.insert(leaf.function.clone(), leaf.module.clone());
        let entry = selfs.entry(leaf.function.clone()).or_default();
        entry.0 += 1;
        *entry.1.entry((leaf.file.clone(), leaf.line)).or_default() += 1;
        let mut seen: Vec<String> = vec![leaf.function];
        for &ip in &s.ips[1..usize::from(s.n)] {
            let r = sym.resolve(ip, true);
            if !seen.contains(&r.function) {
                seen.push(r.function);
            }
        }
        for f in seen {
            *incl.entry(f).or_default() += 1;
        }
    }
    let mut table = String::from("function\tself\tincl\tfile\tline\tmodule\n");
    {
        let mut names: Vec<&String> = selfs.keys().chain(incl.keys()).collect();
        names.sort();
        names.dedup();
        for f in names {
            let (n, lines) = selfs.get(f).map_or((0, None), |(n, l)| (*n, Some(l)));
            let (file, line) = lines
                .and_then(|l| l.iter().max_by_key(|(_, c)| **c).map(|(k, _)| k.clone()))
                .unwrap_or_default();
            let _ = writeln!(
                table,
                "{}\t{n}\t{}\t{file}\t{line}\t{}",
                f.replace(['\t', '\n'], " "),
                incl.get(f).copied().unwrap_or(0),
                fn_module.get(f).map_or("", String::as_str)
            );
        }
    }
    let busy = samples.len() - waiting;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "profile {name}: {} samples of threads that used CPU since the last look, {busy} not in a wait, {waiting} in a wait; 1 ms period",
        samples.len()
    );
    let _ = writeln!(out, "\n== by module (self, not waiting) ==");
    let mut m: Vec<_> = modules.into_iter().collect();
    m.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (module, n) in m.iter().take(12) {
        let _ = writeln!(
            out,
            "{n:>7} {:>5.1}%  {module}",
            100.0 * *n as f64 / busy.max(1) as f64
        );
    }
    let _ = writeln!(out, "\n== by thread (all samples) ==");
    let mut t: Vec<_> = by_thread.into_iter().collect();
    t.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (tid, n) in t.iter().take(12) {
        let _ = writeln!(out, "{n:>7}  tid {tid}  {}", thread_name(*tid));
    }
    let _ = writeln!(
        out,
        "\n== self time: top 40 functions (file:line of the hottest instruction) =="
    );
    let mut s: Vec<_> = selfs.into_iter().collect();
    s.sort_by_key(|a| std::cmp::Reverse(a.1.0));
    for (function, (n, lines)) in s.iter().take(40) {
        let (file, line) = lines
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(k, _)| k.clone())
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{n:>7} {:>5.1}%  {function}  {file}:{line}",
            100.0 * *n as f64 / busy.max(1) as f64
        );
    }
    let _ = writeln!(out, "\n== inclusive time: top 40 functions ==");
    let mut i: Vec<_> = incl.into_iter().collect();
    i.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (function, n) in i.iter().take(40) {
        let _ = writeln!(
            out,
            "{n:>7} {:>5.1}%  {function}",
            100.0 * *n as f64 / busy.max(1) as f64
        );
    }
    (out, table)
}
