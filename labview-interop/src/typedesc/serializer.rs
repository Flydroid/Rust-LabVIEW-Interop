//! Binary serializer for LabVIEW type descriptors — the reverse of
//! [`super::parser`].
//!
//! Ports the section-framing rules from h5labview's `H5LVquery_type`
//! (`typeconv.c:583–860`): each section is
//! `[u16 length][u16 flags<<8|code][payload][optional pascal-string name]`,
//! padded to an even length, where the length includes the length field
//! itself and any padding.
//!
//! The output parses back with [`super::parser::parse`] /
//! [`super::parser::parse_native`] and can be fed to LabVIEW's
//! `TypeToVariant.vi` (big-endian flattened form) to create empty typed
//! Variants for the write-into-variant path.
//!
//! Caveat: for [`TypeDescriptor::Timestamp`] the parser skips an embedded
//! cluster block whose exact contents LabVIEW defines but we do not model;
//! the serializer emits a minimal empty block. This round-trips through our
//! parser but has not yet been validated against `TypeToVariant.vi` —
//! compare against a captured golden vector before relying on it.

use super::parser::ByteOrder;
use super::types::TypeDescriptor;

impl TypeDescriptor {
    /// Serialize to the big-endian (flattened / `Flatten To String`) form.
    ///
    /// This is the form LabVIEW-side VIs exchange as a "typestr" and the
    /// input format of `TypeToVariant.vi`.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_bytes_with_order(ByteOrder::BigEndian)
    }

    /// Serialize with an explicit byte order. Use
    /// [`ByteOrder::NativeEndian`] to mirror the in-memory form returned by
    /// `GetTypeFromLvVariant`.
    pub fn to_bytes_with_order(&self, byte_order: ByteOrder) -> Vec<u8> {
        let mut buf = Vec::new();
        write_section(&mut buf, self, byte_order);
        buf
    }
}

fn put_u16(buf: &mut Vec<u8>, value: u16, order: ByteOrder) {
    match order {
        ByteOrder::BigEndian => buf.extend_from_slice(&value.to_be_bytes()),
        ByteOrder::NativeEndian => buf.extend_from_slice(&value.to_ne_bytes()),
    }
}

fn put_i16(buf: &mut Vec<u8>, value: i16, order: ByteOrder) {
    put_u16(buf, value as u16, order);
}

fn put_pascal_str(buf: &mut Vec<u8>, s: &str) {
    let len = s.len().min(u8::MAX as usize);
    buf.push(len as u8);
    buf.extend_from_slice(&s.as_bytes()[..len]);
}

fn write_section(buf: &mut Vec<u8>, td: &TypeDescriptor, order: ByteOrder) {
    let start = buf.len();
    // Reserve the length field; backpatched below.
    put_u16(buf, 0, order);

    let flags: u8 = if td.name().is_some() { 0x40 } else { 0x00 };
    let code = td.type_code() as u8;
    put_u16(buf, ((flags as u16) << 8) | code as u16, order);

    match td {
        TypeDescriptor::Numeric { .. }
        | TypeDescriptor::Boolean { .. }
        | TypeDescriptor::Variant { .. }
        | TypeDescriptor::Void => {}

        TypeDescriptor::Enum { members, .. } => {
            put_u16(buf, members.len() as u16, order);
            for member in members {
                put_pascal_str(buf, member);
            }
        }

        TypeDescriptor::PhysicalQuantity { units, .. } => {
            put_u16(buf, units.len() as u16, order);
            for unit in units {
                put_u16(buf, unit.unit, order);
                put_i16(buf, unit.power, order);
            }
        }

        TypeDescriptor::String { .. } | TypeDescriptor::Path { .. } => {
            buf.extend_from_slice(&[0xFF; 4]);
        }

        TypeDescriptor::Array { ndims, element, .. } => {
            put_u16(buf, *ndims, order);
            for _ in 0..*ndims {
                buf.extend_from_slice(&[0xFF; 4]);
            }
            write_section(buf, element, order);
        }

        TypeDescriptor::Cluster { fields, .. } => {
            put_u16(buf, fields.len() as u16, order);
            for field in fields {
                write_section(buf, field, order);
            }
        }

        TypeDescriptor::Timestamp { .. } => {
            put_u16(buf, 6, order); // waveform subcode 6 = timestamp
            // Minimal embedded cluster block: length 2 = just the length
            // field, nothing for the parser to skip. See module caveat.
            put_u16(buf, 2, order);
        }

        TypeDescriptor::Waveform { subcode, .. } => {
            put_u16(buf, *subcode as u16, order);
        }

        TypeDescriptor::Refnum {
            kind, referenced, ..
        } => {
            put_u16(buf, *kind, order);
            if let Some(referenced) = referenced {
                write_section(buf, referenced, order);
            }
        }

        TypeDescriptor::Set { element, .. } => {
            write_section(buf, element, order);
        }

        TypeDescriptor::Map { key, value, .. } => {
            put_u16(buf, 2, order); // n_types, always 2
            write_section(buf, key, order);
            write_section(buf, value, order);
        }
    }

    if let Some(name) = td.name() {
        put_pascal_str(buf, name);
    }

    // Pad to even length.
    if (buf.len() - start) % 2 != 0 {
        buf.push(0x00);
    }

    // Backpatch the section length (includes the length field and padding).
    let total = (buf.len() - start) as u16;
    let len_bytes = match order {
        ByteOrder::BigEndian => total.to_be_bytes(),
        ByteOrder::NativeEndian => total.to_ne_bytes(),
    };
    buf[start] = len_bytes[0];
    buf[start + 1] = len_bytes[1];
}

#[cfg(test)]
mod tests {
    use super::super::parser::{parse, parse_with_order};
    use super::super::types::{LvTypeCode, PhysicalUnit, TypeDescriptor};
    use super::*;

    fn num(code: LvTypeCode) -> TypeDescriptor {
        TypeDescriptor::Numeric { code, name: None }
    }

    fn named(td: TypeDescriptor, name: &str) -> TypeDescriptor {
        match td {
            TypeDescriptor::Numeric { code, .. } => TypeDescriptor::Numeric {
                code,
                name: Some(name.to_string()),
            },
            TypeDescriptor::Boolean { .. } => TypeDescriptor::Boolean {
                name: Some(name.to_string()),
            },
            TypeDescriptor::String { .. } => TypeDescriptor::String {
                name: Some(name.to_string()),
            },
            other => other,
        }
    }

    /// Every descriptor shape the crate supports, exercised in one list.
    fn round_trip_fixtures() -> Vec<TypeDescriptor> {
        vec![
            num(LvTypeCode::U32),
            named(num(LvTypeCode::Dbl), "temperature"),
            TypeDescriptor::Boolean { name: None },
            named(TypeDescriptor::String { name: None }, "label"),
            TypeDescriptor::Path { name: None },
            TypeDescriptor::Void,
            TypeDescriptor::Variant { name: None },
            TypeDescriptor::Timestamp { name: None },
            TypeDescriptor::Waveform {
                subcode: 0x03,
                name: None,
            },
            TypeDescriptor::Enum {
                base_type: LvTypeCode::EnumU16,
                members: vec!["A".into(), "B".into(), "C".into()],
                name: Some("mode".into()),
            },
            TypeDescriptor::PhysicalQuantity {
                base_type: LvTypeCode::PhysDbl,
                units: vec![
                    PhysicalUnit { unit: 3, power: 1 },
                    PhysicalUnit { unit: 2, power: -2 },
                ],
                name: None,
            },
            TypeDescriptor::Array {
                ndims: 2,
                element: Box::new(named(num(LvTypeCode::Dbl), "Numeric")),
                name: None,
            },
            TypeDescriptor::Cluster {
                fields: vec![
                    named(TypeDescriptor::String { name: None }, "label"),
                    named(num(LvTypeCode::I32), "value"),
                    named(TypeDescriptor::Boolean { name: None }, "active"),
                ],
                name: Some("record".into()),
            },
            // Nested: cluster of array of cluster
            TypeDescriptor::Cluster {
                fields: vec![
                    named(num(LvTypeCode::U8), "id"),
                    TypeDescriptor::Array {
                        ndims: 1,
                        element: Box::new(TypeDescriptor::Cluster {
                            fields: vec![
                                named(num(LvTypeCode::U64), "big"),
                                named(num(LvTypeCode::U8), "small"),
                            ],
                            name: Some("Tail".into()),
                        }),
                        name: Some("items".into()),
                    },
                ],
                name: None,
            },
            TypeDescriptor::Set {
                element: Box::new(num(LvTypeCode::Dbl)),
                name: None,
            },
            TypeDescriptor::Map {
                key: Box::new(TypeDescriptor::String { name: None }),
                value: Box::new(num(LvTypeCode::I32)),
                name: None,
            },
            TypeDescriptor::Refnum {
                kind: 0x0020,
                referenced: Some(Box::new(TypeDescriptor::Array {
                    ndims: 2,
                    element: Box::new(named(num(LvTypeCode::Dbl), "Numeric")),
                    name: None,
                })),
                name: Some("EDVR_Ref".into()),
            },
            TypeDescriptor::Refnum {
                kind: 1,
                referenced: None,
                name: Some("queue".into()),
            },
        ]
    }

    #[test]
    fn round_trip_big_endian() {
        for td in round_trip_fixtures() {
            let bytes = td.to_bytes();
            let parsed = parse(&bytes)
                .unwrap_or_else(|e| panic!("failed to re-parse {td:?}: {e}"));
            assert_eq!(parsed, td, "big-endian round trip mismatch");
        }
    }

    #[test]
    fn round_trip_native_endian() {
        for td in round_trip_fixtures() {
            let bytes = td.to_bytes_with_order(ByteOrder::NativeEndian);
            let parsed = parse_with_order(&bytes, ByteOrder::NativeEndian)
                .unwrap_or_else(|e| panic!("failed to re-parse {td:?}: {e}"));
            assert_eq!(parsed, td, "native-endian round trip mismatch");
        }
    }

    #[test]
    fn sections_have_even_length() {
        for td in round_trip_fixtures() {
            let bytes = td.to_bytes();
            assert_eq!(bytes.len() % 2, 0, "odd section length for {td:?}");
            let declared = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
            assert_eq!(declared, bytes.len(), "length field mismatch for {td:?}");
        }
    }

    /// Serializing the parsed real EDVR descriptor must re-parse to the same
    /// tree (byte-for-byte identity is NOT required — padding/ordering may
    /// legitimately differ from LabVIEW's own encoding).
    #[test]
    #[cfg(target_endian = "little")]
    fn edvr_descriptor_reserializes_equivalently() {
        use super::super::parser::parse_native;
        #[rustfmt::skip]
        let real: [u8; 42] = [
            0x2A, 0x00, 0x70, 0x40, 0x20, 0x00,
            0x1A, 0x00, 0x40, 0x00, 0x02, 0x00,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0x0C, 0x00, 0x0A, 0x40,
            0x07, b'N', b'u', b'm', b'e', b'r', b'i', b'c',
            0x08, b'E', b'D', b'V', b'R', b'_', b'R', b'e', b'f',
            0x00,
        ];
        let td = parse_native(&real).unwrap();
        let bytes = td.to_bytes_with_order(ByteOrder::NativeEndian);
        let reparsed = parse_with_order(&bytes, ByteOrder::NativeEndian).unwrap();
        assert_eq!(reparsed, td);
        // For this particular descriptor our encoding happens to match
        // LabVIEW's byte-for-byte — lock that in as a golden check.
        assert_eq!(bytes, real);
    }
}
