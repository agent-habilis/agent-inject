//! Emit `AGENT_INJECT_EMBED_FINGERPRINT`, a hash over `skills/`, so that an
//! edit to a `SKILL.md` rebuilds the binary. Stable Rust does not track the
//! files behind `include_dir!`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

const SKIP: &[&str] = include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/embed_skip.rs"));

fn main() {
    let mut hasher = DefaultHasher::new();
    // `skills/` sits at the workspace root, two levels above this package.
    // Read at script runtime, not via `env!`, so a moved checkout does not keep
    // hashing the old location.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let skills = Path::new(&manifest_dir).join("../../skills");
    println!("cargo:rerun-if-changed={}", skills.display());
    hash_dir(&skills, &mut hasher);
    println!(
        "cargo:rustc-env=AGENT_INJECT_EMBED_FINGERPRINT={:016x}",
        hasher.finish()
    );
}

/// Paths and bytes, in a sorted order so the hash is stable across builds.
fn hash_dir(dir: &Path, hasher: &mut DefaultHasher) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if SKIP.iter().any(|skip| entry.file_name() == **skip) {
            continue;
        }
        let path = entry.path();
        path.to_string_lossy().hash(hasher);
        if path.is_dir() {
            hash_dir(&path, hasher);
        } else if let Ok(bytes) = std::fs::read(&path) {
            bytes.hash(hasher);
        }
    }
}
