//! The Keyboard shortcuts dialog (`ui/shortcuts.slint`) lists exactly the
//! keys of the README "Keyboard" table: every key cell there is a row of
//! the dialog, word for word, and the dialog has no other key.

use std::fs;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The first cell of each row of the table under `## Keyboard`.
fn readme_keys() -> Vec<String> {
    let readme = read("README.md");
    let section = readme
        .split("\n## Keyboard\n")
        .nth(1)
        .expect("a Keyboard section in the README");
    let section = section.split("\n## ").next().unwrap();
    section
        .lines()
        .filter(|l| l.starts_with('|'))
        .skip(2) // the header and its rule
        .map(|l| {
            l.trim_start_matches('|')
                .split(" |")
                .next()
                .unwrap()
                .trim()
                .to_string()
        })
        .collect()
}

/// The keys of the dialog's rows: each `@tr("shortcut key" => "…")`.
fn dialog_keys() -> Vec<String> {
    let slint = read("ui/shortcuts.slint");
    let marker = "@tr(\"shortcut key\" => \"";
    slint
        .match_indices(marker)
        .map(|(i, _)| {
            let rest = &slint[i + marker.len()..];
            rest[..rest.find('"').expect("a closing quote")].to_string()
        })
        .collect()
}

#[test]
fn the_dialog_lists_the_readme_keys() {
    let mut readme = readme_keys();
    let mut dialog = dialog_keys();
    assert!(readme.len() > 30, "only {} README rows found", readme.len());
    readme.sort();
    dialog.sort();
    let missing: Vec<_> = readme.iter().filter(|k| !dialog.contains(k)).collect();
    let extra: Vec<_> = dialog.iter().filter(|k| !readme.contains(k)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "README keys missing from the dialog: {missing:?}; dialog keys not in the README: {extra:?}"
    );
    assert_eq!(readme, dialog, "a key appears a different number of times");
}
