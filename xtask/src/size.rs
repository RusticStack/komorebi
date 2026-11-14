//! The release exe size gate (docs/PERF.md, docs/SCOPE.md): komorebi.exe
//! (Komorebi 1.0) at most 21,500,000 bytes. Decimal megabytes, the stripped
//! exe as built by the release profile.

use std::path::{Path, PathBuf};

/// One exe and its ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
    pub exe: &'static str,
    pub max: u64,
}

pub const APP: Gate = Gate {
    exe: "komorebi.exe",
    max: 21_500_000,
};

/// `target/release` under `CARGO_TARGET_DIR` when set, else under the
/// workspace.
pub fn release_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .unwrap_or_else(|| root.join("target"))
        .join("release")
}

/// Checks each gate's exe in `dir`; a missing exe is a failure.
pub fn check(gates: &[Gate], dir: &Path) -> Result<(), String> {
    let mut failures = Vec::new();
    for gate in gates {
        let path = dir.join(gate.exe);
        match std::fs::metadata(&path) {
            Ok(m) => {
                let line = report(gate, m.len());
                println!("size: {line}");
                if m.len() > gate.max {
                    failures.push(line);
                }
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("size: {}", failures.join("; ")))
    }
}

/// `komorebi.exe: 10,667,520 bytes (10.668 MB), ceiling 21,500,000, 10,832,480 left`.
pub fn report(gate: &Gate, len: u64) -> String {
    let tail = if len > gate.max {
        format!("OVER by {}", thousands(len - gate.max))
    } else {
        format!("{} left", thousands(gate.max - len))
    };
    format!(
        "{}: {} bytes ({:.3} MB), ceiling {}, {tail}",
        gate.exe,
        thousands(len),
        len as f64 / 1e6,
        thousands(gate.max),
    )
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceilings_are_the_owners() {
        assert_eq!(APP.max, 21_500_000);
        assert_eq!(APP.exe, "komorebi.exe");
    }

    #[test]
    fn reports_say_what_is_left_or_over() {
        assert_eq!(
            report(&APP, 10_667_520),
            "komorebi.exe: 10,667,520 bytes (10.668 MB), ceiling 21,500,000, 10,832,480 left"
        );
        assert!(report(&APP, 21_500_001).ends_with("OVER by 1"));
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
    }

    #[test]
    fn a_missing_or_large_exe_fails() {
        let dir = std::env::temp_dir().join(format!("komorebi-xtask-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = Gate {
            exe: "small.exe",
            max: 10,
        };
        std::fs::write(dir.join("small.exe"), [0u8; 10]).unwrap();
        assert!(check(&[small], &dir).is_ok());
        std::fs::write(dir.join("small.exe"), [0u8; 11]).unwrap();
        assert!(check(&[small], &dir).is_err());
        let missing = Gate {
            exe: "missing.exe",
            max: 10,
        };
        assert!(check(&[missing], &dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
