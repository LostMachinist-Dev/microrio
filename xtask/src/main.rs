//! Generates `arduino/MicroRIO/src/MicroRIO_protocol.h` from `spec/protocol.toml`.
//!
//! The Rust side gets the same constants through `microrio-proto/build.rs`.
//! Neither output is hand-edited; both come from the one spec, so the firmware
//! and the driver station cannot disagree about the wire format.
//!
//! Run with `cargo xtask gen-header` (see `.cargo/config.toml`).

mod conformance;
mod probe;

use std::{fs, path::PathBuf, process};
use toml::Value;

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "gen-header" => gen_header(),
        "conformance" => {
            let port = std::env::args().nth(2).unwrap_or_else(|| {
                eprintln!("usage: cargo xtask conformance <serial-port>");
                process::exit(2);
            });
            if let Err(e) = conformance::run(&port) {
                eprintln!("\nconformance failed: {e}");
                process::exit(1);
            }
        }
        "probe" => {
            let ip = std::env::args().nth(2).unwrap_or_else(|| {
                eprintln!("usage: cargo xtask probe <ip> [node-id]");
                process::exit(2);
            });
            let node_id = std::env::args()
                .nth(3)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            if let Err(e) = probe::run(&ip, node_id) {
                eprintln!("\nprobe failed: {e}");
                process::exit(1);
            }
        }
        other => {
            eprintln!("unknown task {other:?}; try: gen-header, conformance <port>, probe <ip>");
            process::exit(2);
        }
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

fn gen_header() {
    let root = root();
    let spec_path = root.join("spec/protocol.toml");
    let text = fs::read_to_string(&spec_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", spec_path.display()));
    let spec: Value = text.parse().expect("spec/protocol.toml is not valid TOML");

    let int = |table: &str, key: &str| -> i64 {
        spec.get(table)
            .and_then(|t| t.get(key))
            .and_then(Value::as_integer)
            .unwrap_or_else(|| panic!("spec: missing [{table}].{key}"))
    };

    let mut h = String::new();
    h.push_str(
        "// @generated from spec/protocol.toml by `cargo xtask gen-header` - do not edit.\n\
         //\n\
         // Wire format shared with the Rust driver station. Little-endian, no padding.\n\
         //\n\
         // Header layout (12 bytes):\n\
         //   uint16 magic | uint8 version | uint8 msg_type\n\
         //   uint16 node_id | uint32 schema_hash | uint16 seq\n\n\
         #ifndef MICRORIO_PROTOCOL_H\n\
         #define MICRORIO_PROTOCOL_H\n\n\
         #include <stdint.h>\n\n",
    );

    h.push_str(&format!(
        "// Magic bytes \"MR\".\n#define MICRORIO_MAGIC 0x{:04X}u\n",
        int("protocol", "magic")
    ));
    h.push_str(&format!(
        "// Rejected on mismatch, never negotiated.\n#define MICRORIO_PROTO_VERSION {}u\n",
        int("protocol", "version")
    ));
    h.push_str(&format!(
        "#define MICRORIO_HEADER_LEN {}u\n\n",
        int("protocol", "header_len")
    ));

    h.push_str("// Limits and defaults.\n");
    for (name, key) in [
        ("STR_LEN", "str_len"),
        ("MAX_NAME_LEN", "max_name_len"),
        ("DEFAULT_MAX_FRAME", "default_max_frame"),
        ("MIN_MAX_FRAME", "min_max_frame"),
        ("DEFAULT_RATE_HZ", "default_rate_hz"),
        ("DEFAULT_WATCHDOG_MS", "default_watchdog_ms"),
        ("DEFAULT_HISTORY", "default_history"),
    ] {
        h.push_str(&format!(
            "#define MICRORIO_{name} {}u\n",
            int("limits", key)
        ));
    }
    h.push('\n');

    h.push_str("// Fixed body widths before any payload. See spec/protocol.toml.\n");
    for (name, key) in [
        ("CONTROL_FIXED", "control_fixed"),
        ("TELEM_FIXED", "telem_fixed"),
        ("RECORD_FIXED", "record_fixed"),
    ] {
        h.push_str(&format!("#define MICRORIO_{name} {}u\n", int("frame", key)));
    }
    h.push('\n');

    h.push_str("// Message types.\n");
    for (name, key) in [
        ("SCHEMA_REQ", "schema_req"),
        ("SCHEMA", "schema"),
        ("CONTROL", "control"),
        ("TELEM", "telem"),
    ] {
        h.push_str(&format!(
            "#define MICRORIO_MSG_{name} {}u\n",
            int("msg_type", key)
        ));
    }
    h.push('\n');

    h.push_str("// Channel types. Bools are bitpacked, so their width is not a byte count.\n");
    for key in ["bool", "i32", "f32", "str", "cmd"] {
        let t = spec
            .get("chan_type")
            .and_then(|t| t.get(key))
            .unwrap_or_else(|| panic!("spec: missing [chan_type].{key}"));
        let code = t.get("code").and_then(Value::as_integer).unwrap();
        let size = t.get("size").and_then(Value::as_integer).unwrap();
        h.push_str(&format!(
            "#define MICRORIO_TYPE_{} {code}u  // {size} bytes on the wire\n",
            key.to_uppercase()
        ));
    }
    h.push('\n');

    h.push_str("// Direction. Telemetry is displayed by the GUI; control is populated by it.\n");
    for key in ["telemetry", "control"] {
        h.push_str(&format!(
            "#define MICRORIO_DIR_{} {}u\n",
            key.to_uppercase(),
            int("dir", key)
        ));
    }
    h.push('\n');

    h.push_str("// Operating state. Outputs are live only in ENABLED.\n");
    for key in ["disabled", "enabled", "estop", "fault"] {
        h.push_str(&format!(
            "#define MICRORIO_STATE_{} {}u\n",
            key.to_uppercase(),
            int("state", key)
        ));
    }
    h.push('\n');

    h.push_str("// Control flag bits, station to node.\n");
    for key in ["view_only"] {
        h.push_str(&format!(
            "#define MICRORIO_CFLAG_{} 0x{:02X}u\n",
            key.to_uppercase(),
            int("control_flags", key)
        ));
    }
    h.push('\n');

    h.push_str("// Telemetry flag bits.\n");
    for key in [
        "watchdog_tripped",
        "never_linked",
        "schema_dirty",
        "station_conflict",
    ] {
        h.push_str(&format!(
            "#define MICRORIO_FLAG_{} 0x{:02X}u\n",
            key.to_uppercase(),
            int("flags", key)
        ));
    }
    h.push('\n');

    h.push_str(&format!(
        "// FNV-1a 32-bit, over dir, type, slot and name of each channel in order.\n\
         #define MICRORIO_FNV_OFFSET 0x{:08X}ul\n\
         #define MICRORIO_FNV_PRIME  0x{:08X}ul\n\n",
        int("hash", "fnv_offset"),
        int("hash", "fnv_prime")
    ));

    h.push_str("#endif // MICRORIO_PROTOCOL_H\n");

    let dest = root.join("arduino/MicroRIO/src/MicroRIO_protocol.h");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, h).unwrap_or_else(|e| panic!("cannot write {}: {e}", dest.display()));
    println!("wrote {}", dest.display());
}
