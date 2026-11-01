//! Generates protocol constants from `spec/protocol.toml`.
//!
//! The same spec drives the C++ header, so neither side can drift.

use std::{env, fs, path::PathBuf};
use toml::Value;

fn main() {
    let spec_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("spec")
        .join("protocol.toml");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let text = fs::read_to_string(&spec_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", spec_path.display()));
    let spec: Value = text.parse().expect("spec/protocol.toml is not valid TOML");

    let int = |table: &str, key: &str| -> i64 {
        spec.get(table)
            .and_then(|t| t.get(key))
            .and_then(Value::as_integer)
            .unwrap_or_else(|| panic!("spec: missing [{table}].{key}"))
    };

    let mut out = String::new();
    out.push_str("// @generated from spec/protocol.toml - do not edit.\n\n");

    out.push_str(&format!(
        "/// Magic bytes `MR`, little-endian on the wire.\npub const MAGIC: u16 = 0x{:04X};\n",
        int("protocol", "magic")
    ));
    out.push_str(&format!(
        "/// Protocol version. A mismatch is rejected, never negotiated.\npub const PROTO_VERSION: u8 = {};\n",
        int("protocol", "version")
    ));
    out.push_str(&format!(
        "/// Fixed header width in bytes.\npub const HEADER_LEN: usize = {};\n\n",
        int("protocol", "header_len")
    ));

    for (name, key) in [
        ("STR_LEN", "str_len"),
        ("MAX_NAME_LEN", "max_name_len"),
        ("DEFAULT_MAX_FRAME", "default_max_frame"),
        ("MIN_MAX_FRAME", "min_max_frame"),
    ] {
        out.push_str(&format!(
            "pub const {name}: usize = {};\n",
            int("limits", key)
        ));
    }
    for (name, key) in [
        ("DEFAULT_RATE_HZ", "default_rate_hz"),
        ("DEFAULT_WATCHDOG_MS", "default_watchdog_ms"),
        ("DEFAULT_UI_STALE_MS", "default_ui_stale_ms"),
        ("DEFAULT_HISTORY", "default_history"),
    ] {
        out.push_str(&format!(
            "pub const {name}: u16 = {};\n",
            int("limits", key)
        ));
    }
    for (name, key) in [
        ("CONTROL_FIXED", "control_fixed"),
        ("TELEM_FIXED", "telem_fixed"),
        ("RECORD_FIXED", "record_fixed"),
    ] {
        out.push_str(&format!(
            "pub const {name}: usize = {};\n",
            int("frame", key)
        ));
    }
    out.push('\n');

    out.push_str(&format!(
        "pub const FNV_OFFSET: u32 = 0x{:08X};\npub const FNV_PRIME: u32 = 0x{:08X};\n\n",
        int("hash", "fnv_offset"),
        int("hash", "fnv_prime")
    ));

    out.push_str("pub mod msg {\n");
    for k in ["schema_req", "schema", "control", "telem"] {
        out.push_str(&format!(
            "    pub const {}: u8 = {};\n",
            k.to_uppercase(),
            int("msg_type", k)
        ));
    }
    out.push_str("}\n\n");

    out.push_str("/// Control frame flag bits, station to node.\npub mod ctrl_flag {\n");
    for k in ["view_only"] {
        out.push_str(&format!(
            "    pub const {}: u8 = 0x{:02X};\n",
            k.to_uppercase(),
            int("control_flags", k)
        ));
    }
    out.push_str("}\n\n");

    out.push_str("pub mod flag {\n");
    for k in [
        "watchdog_tripped",
        "never_linked",
        "schema_dirty",
        "station_conflict",
    ] {
        out.push_str(&format!(
            "    pub const {}: u8 = 0x{:02X};\n",
            k.to_uppercase(),
            int("flags", k)
        ));
    }
    out.push_str("}\n");

    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("consts.rs");
    fs::write(&dest, out).expect("cannot write generated consts");
}
