//! Run a program to completion and print its peak working set and peak
//! private memory (`K32GetProcessMemoryInfo`), for measuring the app. The
//! program inherits the environment, so point it at a throwaway cache:
//!
//! ```text
//! $env:KOMOREBI_OFFLINE=1; $env:KOMOREBI_DB="<seeded.sqlite3>"
//! $env:KOMOREBI_SNAPSHOT="<out.bmp>"; $env:KOMOREBI_SNAPSHOT_SELECT=0
//! $env:KOMOREBI_SNAPSHOT_WALK=13
//! cargo run --release --example peak_memory -- target\release\komorebi.exe
//! ```

#[cfg(not(windows))]
fn main() {
    eprintln!("peak_memory: Windows only");
}

#[cfg(windows)]
use anyhow::{Context, Result, bail};

#[cfg(windows)]
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let program = args
        .next()
        .context("usage: peak_memory <program> [args...]")?;
    let mut child = std::process::Command::new(program).args(args).spawn()?;
    let status = child.wait()?;
    // The process object (and its counters) lives as long as `child`'s
    // handle, after the process has exited.
    let Some(c) = counters(&child) else {
        bail!("K32GetProcessMemoryInfo failed");
    };
    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    println!(
        "{status}: peak working set {:.1} MiB, peak private {:.1} MiB",
        mib(c.PeakWorkingSetSize),
        mib(c.PeakPagefileUsage),
    );
    Ok(())
}

#[cfg(windows)]
fn counters(
    child: &std::process::Child,
) -> Option<windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: the handle is the child's, still open; `c` is a valid,
    // writable PROCESS_MEMORY_COUNTERS of the size passed.
    let ok = unsafe { K32GetProcessMemoryInfo(child.as_raw_handle(), &mut c, c.cb) };
    (ok != 0).then_some(c)
}
