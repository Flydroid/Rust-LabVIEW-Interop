//! Golden-vector regression tests for the type descriptor parser.
//!
//! Records in `tests/golden/*.tsv` are captured from a real LabVIEW runtime
//! by `Capture Golden Vectors.vi` (see the Variant Tests project). Each line
//! is tab-separated:
//!
//! ```text
//! name <TAB> typedesc_native_hex <TAB> typestr_flattened_hex <TAB> canonical
//! ```
//!
//! - `typedesc_native_hex` — raw `GetTypeFromLvVariant` bytes (host-native
//!   endianness; all supported LabVIEW hosts are little-endian) from the
//!   `variant_typedesc_hex` test export.
//! - `typestr_flattened_hex` — the big-endian typestr from
//!   `Variant To Flattened String`.
//! - `canonical` — the expected canonical value string (used by the LabVIEW
//!   test VIs; informational here).
//!
//! Empty fields are skipped. Lines starting with `#` are comments.
//!
//! For every record this test asserts:
//! 1. present native bytes parse via `parse_native`,
//! 2. present flattened bytes parse via `parse`,
//! 3. when both are present, the two parses agree structurally
//!    (field names may legitimately differ between the two access paths,
//!    so names are compared only when both sides carry them),
//! 4. each parsed tree survives a serialize → re-parse round trip.
//!
//! This turns one manual LabVIEW capture run into permanent LabVIEW-free
//! CI coverage against real-world bytes.

use labview_interop::typedesc::{parse, parse_native, parse_with_order, ByteOrder, TypeDescriptor};

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err(format!("odd hex length {}", s.len()));
    }
    (0..s.len() / 2)
        .map(|i| {
            u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
                .map_err(|e| format!("bad hex at byte {i}: {e}"))
        })
        .collect()
}

/// Structural comparison that tolerates differing element names between the
/// in-memory and flattened access paths (both trees must otherwise match).
fn structurally_equal(a: &TypeDescriptor, b: &TypeDescriptor) -> bool {
    use TypeDescriptor as TD;
    match (a, b) {
        (TD::Numeric { code: ca, .. }, TD::Numeric { code: cb, .. }) => ca == cb,
        (TD::Boolean { .. }, TD::Boolean { .. })
        | (TD::String { .. }, TD::String { .. })
        | (TD::Path { .. }, TD::Path { .. })
        | (TD::Variant { .. }, TD::Variant { .. })
        | (TD::Timestamp { .. }, TD::Timestamp { .. })
        | (TD::Void, TD::Void) => true,
        (
            TD::Enum {
                base_type: ba,
                members: ma,
                ..
            },
            TD::Enum {
                base_type: bb,
                members: mb,
                ..
            },
        ) => ba == bb && ma == mb,
        (
            TD::Array {
                ndims: na,
                element: ea,
                ..
            },
            TD::Array {
                ndims: nb,
                element: eb,
                ..
            },
        ) => na == nb && structurally_equal(ea, eb),
        (TD::Cluster { fields: fa, .. }, TD::Cluster { fields: fb, .. }) => {
            fa.len() == fb.len()
                && fa
                    .iter()
                    .zip(fb.iter())
                    .all(|(x, y)| structurally_equal(x, y))
        }
        (TD::Waveform { subcode: sa, .. }, TD::Waveform { subcode: sb, .. }) => sa == sb,
        (
            TD::PhysicalQuantity {
                base_type: ba,
                units: ua,
                ..
            },
            TD::PhysicalQuantity {
                base_type: bb,
                units: ub,
                ..
            },
        ) => ba == bb && ua == ub,
        (
            TD::Refnum {
                kind: ka,
                referenced: ra,
                ..
            },
            TD::Refnum {
                kind: kb,
                referenced: rb,
                ..
            },
        ) => {
            ka == kb
                && match (ra, rb) {
                    (Some(x), Some(y)) => structurally_equal(x, y),
                    (None, None) => true,
                    _ => false,
                }
        }
        (TD::Set { element: ea, .. }, TD::Set { element: eb, .. }) => structurally_equal(ea, eb),
        (
            TD::Map {
                key: ka, value: va, ..
            },
            TD::Map {
                key: kb, value: vb, ..
            },
        ) => structurally_equal(ka, kb) && structurally_equal(va, vb),
        _ => false,
    }
}

fn round_trips(td: &TypeDescriptor, order: ByteOrder, context: &str) {
    let bytes = td.to_bytes_with_order(order);
    let reparsed = parse_with_order(&bytes, order)
        .unwrap_or_else(|e| panic!("{context}: serialized bytes failed to re-parse: {e}"));
    assert_eq!(&reparsed, td, "{context}: serialize/re-parse round trip");
}

#[test]
fn golden_vectors() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden");

    let mut checked = 0usize;

    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => {
            eprintln!(
                "golden vector directory {dir:?} missing — run Capture Golden Vectors.vi \
                 and commit the output"
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("tsv") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"));

        for (line_no, line) in content.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let name = fields[0];
            let context = format!("{}:{} ({name})", path.display(), line_no + 1);
            let native_hex = fields.get(1).copied().unwrap_or("");
            let flattened_hex = fields.get(2).copied().unwrap_or("");

            let native_td = if native_hex.is_empty() {
                None
            } else {
                let bytes = hex_decode(native_hex)
                    .unwrap_or_else(|e| panic!("{context}: native hex: {e}"));
                let td = parse_native(&bytes)
                    .unwrap_or_else(|e| panic!("{context}: parse_native failed: {e}"));
                round_trips(&td, ByteOrder::NativeEndian, &context);
                Some(td)
            };

            let flattened_td = if flattened_hex.is_empty() {
                None
            } else {
                let bytes = hex_decode(flattened_hex)
                    .unwrap_or_else(|e| panic!("{context}: flattened hex: {e}"));
                let td =
                    parse(&bytes).unwrap_or_else(|e| panic!("{context}: parse failed: {e}"));
                round_trips(&td, ByteOrder::BigEndian, &context);
                Some(td)
            };

            if let (Some(native), Some(flattened)) = (&native_td, &flattened_td) {
                assert!(
                    structurally_equal(native, flattened),
                    "{context}: native and flattened descriptors disagree:\n\
                     native:    {native:?}\nflattened: {flattened:?}"
                );
            }

            if native_td.is_some() || flattened_td.is_some() {
                checked += 1;
            }
        }
    }

    println!("checked {checked} golden type descriptor records");
    assert!(
        checked > 0,
        "golden vector directory exists but contains no records"
    );
}
