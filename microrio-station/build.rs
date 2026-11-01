//! Stamps the build with the commit it came from.
//!
//! An operator reporting a problem can read the version off the window, and
//! it names an exact commit rather than a version number that changes once a
//! release. Built outside a git checkout, it says so rather than lying.

use std::process::Command;

fn main() {
    // Rebuild when the checked-out commit changes. Cargo cannot watch the
    // working tree, so a release build passes MICRORIO_STAMP to force this
    // to run again and catch uncommitted changes in the mark.
    for p in ["../.git/HEAD", "../.git/refs/heads"] {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-env-changed=MICRORIO_STAMP");

    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then_some(s)
    };

    let commit = git(&["describe", "--always", "--dirty=+", "--tags"])
        .unwrap_or_else(|| "no-git".to_string());
    let date = git(&["log", "-1", "--date=short", "--format=%cd"])
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MICRORIO_BUILD={commit}");
    println!("cargo:rustc-env=MICRORIO_BUILD_DATE={date}");
}
