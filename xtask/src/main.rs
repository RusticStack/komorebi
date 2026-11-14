//! Komorebi's development tasks (docs/GPUI-TESTING.md), modelled on Skua's
//! `cargo xtask check`:
//!
//! - `cargo xtask check [--release] [--all]`: the one gate before a commit:
//!   `cargo fmt --check`, clippy over the workspace with warnings as errors,
//!   the app's, the library's and this crate's tests, the GPUI lints, and the
//!   exe's size. `--release` builds the release exe first (long: fat LTO);
//!   without it the size of an existing release build is checked, or the
//!   step says it was skipped. `--all` also runs every test of the
//!   workspace's default member, the library's integration tests included
//!   (`cargo test --locked`, long).
//! - `cargo xtask lint`: the token lint and the no-timers lint (lint.rs).
//! - `cargo xtask size`: the release `komorebi.exe` against its ceiling
//!   (size.rs).

mod lint;
mod size;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check") => check(&args[1..]),
        Some("lint") => lint::run(&workspace_root()),
        Some("size") => size_command(&args[1..]),
        Some(other) => Err(format!(
            "unknown xtask `{other}` (expected: check, lint, size)"
        )),
        None => Err("usage: cargo xtask <check [--release] [--all] | lint | size>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Every gate in order; stops at the first failure.
fn check(args: &[String]) -> Result<(), String> {
    let release = args.iter().any(|a| a == "--release");
    let all = args.iter().any(|a| a == "--all");
    if let Some(bad) = args.iter().find(|a| *a != "--release" && *a != "--all") {
        return Err(format!("check: unknown option {bad}"));
    }
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "-p", "komorebi-gpui", "--locked"])?;
    cargo(&["test", "-p", "komorebi", "--lib", "--locked"])?;
    cargo(&["test", "-p", "xtask", "--locked"])?;
    if all {
        cargo(&["test", "--locked"])?;
    }
    lint::run(&workspace_root())?;
    if release {
        cargo(&["build", "-p", "komorebi-gpui", "--release", "--locked"])?;
    }
    let exe = size::release_dir(&workspace_root()).join(size::APP.exe);
    if exe.exists() {
        size::check(&[size::APP], &size::release_dir(&workspace_root()))?;
        println!("xtask check: ok");
    } else {
        println!(
            "size: SKIPPED, no release build at {} (run `cargo xtask check --release`)",
            exe.display()
        );
        println!("xtask check: ok, except the size check (skipped)");
    }
    Ok(())
}

fn size_command(args: &[String]) -> Result<(), String> {
    if let Some(other) = args.first() {
        return Err(format!("size: unknown option {other}"));
    }
    size::check(&[size::APP], &size::release_dir(&workspace_root()))
}

/// The workspace root: the parent of this crate's folder.
fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

/// Runs cargo (the one running this task) in the workspace root.
fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    println!("xtask: cargo {}", args.join(" "));
    let status = Command::new(&cargo)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("could not run cargo {}: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!("cargo {} failed ({status})", args.join(" ")));
    }
    Ok(())
}
