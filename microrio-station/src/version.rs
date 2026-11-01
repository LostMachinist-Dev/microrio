//! What this build is.
//!
//! Shown in the window and printed by `--version`, so a report of odd
//! behaviour can be tied to an exact commit rather than to "the latest one".

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `git describe` of the checkout this was built from, `+` if it had
/// uncommitted changes, or `no-git` when built outside a checkout.
pub const COMMIT: &str = env!("MICRORIO_BUILD");
pub const DATE: &str = env!("MICRORIO_BUILD_DATE");

/// For the corner of the window.
pub fn short() -> String {
    format!("v{VERSION} · {COMMIT}")
}

/// For `--version` and the hover text.
pub fn full() -> String {
    format!("MicroRIO station v{VERSION} · build {COMMIT} · {DATE}")
}
