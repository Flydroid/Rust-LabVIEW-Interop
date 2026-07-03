//! Binary parser for LabVIEW type descriptor byte strings.
//!
//! Ports the parsing logic from h5labview's `recurse_typedesc` (`typeconv.c:222–450`)
//! and `parse_array_td` (`typeconv.c:72–86`).
//!
//! The type descriptor format is a sequence of sections, each structured as:
//! ```text
//! [2 bytes: section length]
//! [1 byte:  flags (bit 0x40 = has name)]
//! [1 byte:  type code]
//! [... type-specific payload ...]
//! [optional: pascal string name (1 byte length + chars)]
//! ```
//!
//! Two byte orders are supported:
//! - **Big-endian**: used by `Flatten To String` and h5labview.
//! - **Native endian**: used by `GetTypeFromLvVariant` (in-memory format).

use crate::errors::{InternalError, LVInteropError};

use super::types::{LvTypeCode, PhysicalUnit, TypeDescriptor};

/// Byte order for multi-byte fields in the type descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    BigEndian,
    /// Native endianness of the current platform.
    /// Used for in-memory type descriptors from `GetTypeFromLvVariant`.
    NativeEndian,
}

/// Internal cursor for parsing type descriptor bytes.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    byte_order: ByteOrder,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], byte_order: ByteOrder) -> Self {
        Self {
            data,
            pos: 0,
            byte_order,
        }
    }

    fn read_u8(&mut self) -> Result<u8, LVInteropError> {
        if self.pos >= self.data.len() {
            return Err(parse_err("unexpected end of data reading u8"));
        }
        let val = self.data[self.pos];
        self.pos += 1;
        Ok(val)
    }

    fn read_u16(&mut self) -> Result<u16, LVInteropError> {
        if self.pos + 2 > self.data.len() {
            return Err(parse_err("unexpected end of data reading u16"));
        }
        let bytes = [self.data[self.pos], self.data[self.pos + 1]];
        let val = match self.byte_order {
            ByteOrder::BigEndian => u16::from_be_bytes(bytes),
            ByteOrder::NativeEndian => u16::from_ne_bytes(bytes),
        };
        self.pos += 2;
        Ok(val)
    }

    fn read_i16(&mut self) -> Result<i16, LVInteropError> {
        if self.pos + 2 > self.data.len() {
            return Err(parse_err("unexpected end of data reading i16"));
        }
        let bytes = [self.data[self.pos], self.data[self.pos + 1]];
        let val = match self.byte_order {
            ByteOrder::BigEndian => i16::from_be_bytes(bytes),
            ByteOrder::NativeEndian => i16::from_ne_bytes(bytes),
        };
        self.pos += 2;
        Ok(val)
    }

    /// Read a pascal string: 1 byte length prefix + that many bytes of UTF-8 content.
    fn read_pascal_str(&mut self) -> Result<String, LVInteropError> {
        let len = self.read_u8()? as usize;
        if self.pos + len > self.data.len() {
            return Err(parse_err("unexpected end of data reading pascal string"));
        }
        let s = std::str::from_utf8(&self.data[self.pos..self.pos + len])
            .map_err(|e| parse_err(&format!("invalid UTF-8 in pascal string: {e}")))?
            .to_string();
        self.pos += len;
        Ok(s)
    }

    /// Skip `n` bytes forward.
    fn skip(&mut self, n: usize) -> Result<(), LVInteropError> {
        if self.pos + n > self.data.len() {
            return Err(parse_err("unexpected end of data while skipping"));
        }
        self.pos += n;
        Ok(())
    }
}

fn parse_err(msg: &str) -> LVInteropError {
    InternalError::InvalidTypeDescriptor(msg.to_string()).into()
}

/// Parse a single type descriptor section starting at the cursor's current position.
///
/// This is the Rust port of h5labview's `recurse_typedesc`.
/// After parsing, the cursor is advanced past the entire section (to `start + section_length`).
fn parse_section(cursor: &mut Cursor) -> Result<TypeDescriptor, LVInteropError> {
    let section_start = cursor.pos;
    let section_len = cursor.read_u16()? as usize;

    if section_len < 4 {
        return Err(parse_err(&format!(
            "section length {section_len} is too small (minimum 4)"
        )));
    }

    // The section_len includes these 2 bytes of the length field itself? No.
    // In h5labview: `len = get16be(&p)` then `*typestr += len` (original pointer moved by `len`).
    // The length field is read first, then the pointer is advanced by `len` bytes from after
    // the length field? Actually looking at the code more carefully:
    //
    // ```c
    // char *p = *typestr;
    // size_t len = get16be(&p);     // p now points 2 bytes in
    // uint8_t hasname = 0x40 & *p++;
    // uint16_t code = *p++;
    // *typestr += len;              // advance ORIGINAL pointer by len (from start)
    // ```
    //
    // So `len` is the total section length INCLUDING the 2-byte length field itself.
    let section_end = section_start + section_len;

    if section_end > cursor.data.len() {
        return Err(parse_err(&format!(
            "section extends beyond input: end={section_end}, data_len={}",
            cursor.data.len()
        )));
    }

    // The flags/code pair is a single u16 (`flags << 8 | code`) stored in the
    // descriptor's byte order. Big-endian (flattened) descriptors therefore lay
    // out `[flags][code]`, while native little-endian descriptors from
    // `GetTypeFromLvVariant` lay out `[code][flags]` — verified empirically
    // against a LabVIEW 2025 x64 EDVR descriptor (see
    // docs/variant-implementation-plan.md, "Verified Findings").
    let flags_code = cursor.read_u16()?;
    let flags = (flags_code >> 8) as u8;
    let has_name = (flags & 0x40) != 0;
    let code_byte = (flags_code & 0xFF) as u8;

    let code = LvTypeCode::try_from(code_byte)
        .map_err(|_| parse_err(&format!("unknown type code: 0x{code_byte:02X}")))?;

    let td = match code {
        // Numeric types 0x01-0x0E
        LvTypeCode::I8
        | LvTypeCode::I16
        | LvTypeCode::I32
        | LvTypeCode::I64
        | LvTypeCode::U8
        | LvTypeCode::U16
        | LvTypeCode::U32
        | LvTypeCode::U64
        | LvTypeCode::Sgl
        | LvTypeCode::Dbl
        | LvTypeCode::Ext
        | LvTypeCode::CSgl
        | LvTypeCode::CDbl
        | LvTypeCode::CExt => {
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Numeric { code, name }
        }

        // Physical quantities 0x19-0x1E
        LvTypeCode::PhysSgl
        | LvTypeCode::PhysDbl
        | LvTypeCode::PhysExt
        | LvTypeCode::PhysCSgl
        | LvTypeCode::PhysCDbl
        | LvTypeCode::PhysCExt => {
            let num_units = cursor.read_u16()? as usize;
            let mut units = Vec::with_capacity(num_units);
            for _ in 0..num_units {
                let unit = cursor.read_u16()?;
                let power = cursor.read_i16()?;
                units.push(PhysicalUnit { unit, power });
            }
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::PhysicalQuantity {
                base_type: code,
                units,
                name,
            }
        }

        // Enums 0x15-0x17
        LvTypeCode::EnumU8 | LvTypeCode::EnumU16 | LvTypeCode::EnumU32 => {
            let member_count = cursor.read_u16()? as usize;
            let mut members = Vec::with_capacity(member_count);
            for _ in 0..member_count {
                members.push(cursor.read_pascal_str()?);
            }
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Enum {
                base_type: code,
                members,
                name,
            }
        }

        // Boolean 0x21
        LvTypeCode::Boolean => {
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Boolean { name }
        }

        // String 0x30 — skip 4-byte 0xFFFFFFFF marker
        LvTypeCode::String => {
            cursor.skip(4)?; // 0xFFFFFFFF
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::String { name }
        }

        // Path 0x32 — skip 4-byte 0xFFFFFFFF marker
        LvTypeCode::Path => {
            cursor.skip(4)?; // 0xFFFFFFFF
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Path { name }
        }

        // Array 0x40
        LvTypeCode::Array => {
            let ndims = cursor.read_u16()?;
            // Skip ndims × 0xFFFFFFFF dimension placeholders
            cursor.skip(ndims as usize * 4)?;
            // Recurse for element type
            let element = parse_section(cursor)?;
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Array {
                ndims,
                element: Box::new(element),
                name,
            }
        }

        // Cluster 0x50
        LvTypeCode::Cluster => {
            let field_count = cursor.read_u16()? as usize;
            let mut fields = Vec::with_capacity(field_count);
            for _ in 0..field_count {
                fields.push(parse_section(cursor)?);
            }
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Cluster { fields, name }
        }

        // Variant 0x53
        LvTypeCode::Variant => {
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Variant { name }
        }

        // Waveform 0x54
        LvTypeCode::Waveform => {
            let subcode = cursor.read_u16()? as u8;
            if subcode == 6 {
                // Timestamp — skip the cluster contents
                // h5labview: `p += get16be(&p)` to skip cluster section
                let cluster_len = cursor.read_u16()? as usize;
                cursor.skip(cluster_len.saturating_sub(2))?;
                let name = read_name_if(cursor, has_name, section_end)?;
                TypeDescriptor::Timestamp { name }
            } else {
                let name = read_name_if(cursor, has_name, section_end)?;
                TypeDescriptor::Waveform { subcode, name }
            }
        }

        // Refnum 0x70
        LvTypeCode::Refnum => {
            let kind = cursor.read_u16()?;
            // Data-bearing refnums (e.g. DVRs) embed a nested type descriptor
            // for the referenced type. Other refnum kinds have no payload
            // beyond the kind word. Only one empirical sample exists (external
            // DVR, kind 0x0020), so detect a nested section conservatively:
            // it must announce a plausible length that fits in this section.
            let referenced = if looks_like_nested_section(cursor, section_end) {
                Some(Box::new(parse_section(cursor)?))
            } else {
                None
            };
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Refnum {
                kind,
                referenced,
                name,
            }
        }

        // Set 0x73
        LvTypeCode::Set => {
            let element = parse_section(cursor)?;
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Set {
                element: Box::new(element),
                name,
            }
        }

        // Map 0x74
        LvTypeCode::Map => {
            let _n_types = cursor.read_u16()?; // always 2
            let key = parse_section(cursor)?;
            let value = parse_section(cursor)?;
            let name = read_name_if(cursor, has_name, section_end)?;
            TypeDescriptor::Map {
                key: Box::new(key),
                value: Box::new(value),
                name,
            }
        }

        // Void 0x00
        LvTypeCode::Void => TypeDescriptor::Void,
    };

    // Ensure cursor is at section end (skip any trailing padding)
    cursor.pos = section_end;

    Ok(td)
}

/// Heuristic check whether the bytes at the cursor start a nested type
/// descriptor section (used for refnum payloads, where a name may follow
/// directly instead). A nested section must announce a length of at least 4
/// that fits entirely before `section_end`. A pascal-string name read as a
/// u16 essentially never satisfies this because its second byte is a
/// printable character, producing a huge length in at least one byte order.
fn looks_like_nested_section(cursor: &Cursor, section_end: usize) -> bool {
    let remaining = section_end.saturating_sub(cursor.pos);
    if remaining < 4 {
        return false;
    }
    let bytes = [cursor.data[cursor.pos], cursor.data[cursor.pos + 1]];
    let len = match cursor.byte_order {
        ByteOrder::BigEndian => u16::from_be_bytes(bytes),
        ByteOrder::NativeEndian => u16::from_ne_bytes(bytes),
    } as usize;
    (4..=remaining).contains(&len)
}

/// Read the optional pascal string name if the `has_name` flag is set.
/// Handles potential alignment byte before the name.
fn read_name_if(
    cursor: &mut Cursor,
    has_name: bool,
    section_end: usize,
) -> Result<Option<String>, LVInteropError> {
    if !has_name {
        return Ok(None);
    }
    // From h5labview: name might be aligned — if current byte is 0 and next isn't, skip one.
    if cursor.pos < section_end
        && cursor.data[cursor.pos] == 0
        && cursor.pos + 1 < section_end
        && cursor.data[cursor.pos + 1] != 0
    {
        cursor.pos += 1;
    }
    if cursor.pos >= section_end {
        return Ok(None);
    }
    Ok(Some(cursor.read_pascal_str()?))
}

/// Parse a complete type descriptor from big-endian bytes (e.g. from `Flatten To String`).
///
/// Returns the parsed descriptor. The entire byte slice is consumed as one section.
///
/// # Errors
///
/// Returns `InvalidTypeDescriptor` if the bytes cannot be parsed.
pub fn parse(bytes: &[u8]) -> Result<TypeDescriptor, LVInteropError> {
    parse_with_order(bytes, ByteOrder::BigEndian)
}

/// Parse a type descriptor from in-memory bytes (native endianness).
///
/// Use this for type descriptors obtained from `GetTypeFromLvVariant`,
/// which stores multi-byte fields in the platform's native byte order.
///
/// # Errors
///
/// Returns `InvalidTypeDescriptor` if the bytes cannot be parsed.
pub fn parse_native(bytes: &[u8]) -> Result<TypeDescriptor, LVInteropError> {
    parse_with_order(bytes, ByteOrder::NativeEndian)
}

/// Parse a type descriptor with the specified byte order.
pub fn parse_with_order(
    bytes: &[u8],
    byte_order: ByteOrder,
) -> Result<TypeDescriptor, LVInteropError> {
    if bytes.is_empty() {
        return Err(parse_err("empty type descriptor"));
    }
    let mut cursor = Cursor::new(bytes, byte_order);
    parse_section(&mut cursor)
}

/// Parse type descriptor bytes, detecting if the outermost element is
/// a top-level array wrapper (as used by LabVIEW for array data).
///
/// Returns `(Some(ndims), inner_type)` for arrays, or `(None, type)` for
/// scalars/clusters.
///
/// This mirrors h5labview's `parse_array_td` which checks byte offset 3
/// for the array type code 0x40.
pub fn parse_with_array(bytes: &[u8]) -> Result<(Option<u16>, TypeDescriptor), LVInteropError> {
    let td = parse(bytes)?;
    match td {
        TypeDescriptor::Array { ndims, element, .. } => Ok((Some(ndims), *element)),
        other => Ok((None, other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to build a minimal type descriptor section
    fn make_section(code: u8, payload: &[u8]) -> Vec<u8> {
        let total_len = 4 + payload.len(); // 2 len + 1 flags + 1 code + payload
        let mut buf = Vec::with_capacity(total_len);
        buf.extend_from_slice(&(total_len as u16).to_be_bytes()); // section length
        buf.push(0x00); // flags: no name
        buf.push(code);
        buf.extend_from_slice(payload);
        // Pad to even length if needed
        if buf.len() % 2 != 0 {
            buf.push(0x00);
            // Fix length field
            let new_len = buf.len() as u16;
            buf[0] = (new_len >> 8) as u8;
            buf[1] = new_len as u8;
        }
        buf
    }

    fn make_named_section(code: u8, payload: &[u8], name: &str) -> Vec<u8> {
        let name_bytes_len = 1 + name.len(); // pascal string: 1 byte len + chars
        let total_len = 4 + payload.len() + name_bytes_len;
        let mut buf = Vec::with_capacity(total_len);
        buf.extend_from_slice(&(total_len as u16).to_be_bytes());
        buf.push(0x40); // flags: has name
        buf.push(code);
        buf.extend_from_slice(payload);
        buf.push(name.len() as u8); // pascal string length
        buf.extend_from_slice(name.as_bytes());
        // Pad to even length if needed
        if buf.len() % 2 != 0 {
            buf.push(0x00);
            let new_len = buf.len() as u16;
            buf[0] = (new_len >> 8) as u8;
            buf[1] = new_len as u8;
        }
        buf
    }

    #[test]
    fn parse_scalar_u32() {
        let bytes = make_section(0x07, &[]);
        let td = parse(&bytes).unwrap();
        assert_eq!(
            td,
            TypeDescriptor::Numeric {
                code: LvTypeCode::U32,
                name: None
            }
        );
    }

    #[test]
    fn parse_scalar_with_name() {
        let bytes = make_named_section(0x0A, &[], "temperature");
        let td = parse(&bytes).unwrap();
        assert_eq!(
            td,
            TypeDescriptor::Numeric {
                code: LvTypeCode::Dbl,
                name: Some("temperature".to_string())
            }
        );
    }

    #[test]
    fn parse_boolean() {
        let bytes = make_section(0x21, &[]);
        let td = parse(&bytes).unwrap();
        assert_eq!(td, TypeDescriptor::Boolean { name: None });
    }

    #[test]
    fn parse_string() {
        // String has 4-byte 0xFFFFFFFF marker
        let bytes = make_section(0x30, &[0xFF, 0xFF, 0xFF, 0xFF]);
        let td = parse(&bytes).unwrap();
        assert_eq!(td, TypeDescriptor::String { name: None });
    }

    #[test]
    fn parse_path() {
        let bytes = make_section(0x32, &[0xFF, 0xFF, 0xFF, 0xFF]);
        let td = parse(&bytes).unwrap();
        assert_eq!(td, TypeDescriptor::Path { name: None });
    }

    #[test]
    fn parse_void() {
        let bytes = make_section(0x00, &[]);
        let td = parse(&bytes).unwrap();
        assert_eq!(td, TypeDescriptor::Void);
    }

    #[test]
    fn parse_1d_array_of_dbl() {
        // Array payload: ndims=1, 0xFFFFFFFF, then element section (Dbl)
        let element = make_section(0x0A, &[]);
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_be_bytes()); // ndims = 1
        payload.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes()); // dim placeholder
        payload.extend_from_slice(&element);

        let bytes = make_section(0x40, &payload);
        let td = parse(&bytes).unwrap();

        match td {
            TypeDescriptor::Array {
                ndims,
                element,
                name,
            } => {
                assert_eq!(ndims, 1);
                assert_eq!(
                    *element,
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::Dbl,
                        name: None
                    }
                );
                assert_eq!(name, None);
            }
            _ => panic!("expected Array, got {td:?}"),
        }
    }

    #[test]
    fn parse_with_array_1d() {
        let element = make_section(0x0A, &[]);
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
        payload.extend_from_slice(&element);

        let bytes = make_section(0x40, &payload);
        let (ndims, inner) = parse_with_array(&bytes).unwrap();

        assert_eq!(ndims, Some(1));
        assert_eq!(
            inner,
            TypeDescriptor::Numeric {
                code: LvTypeCode::Dbl,
                name: None
            }
        );
    }

    #[test]
    fn parse_with_array_scalar() {
        let bytes = make_section(0x03, &[]);
        let (ndims, inner) = parse_with_array(&bytes).unwrap();

        assert_eq!(ndims, None);
        assert_eq!(
            inner,
            TypeDescriptor::Numeric {
                code: LvTypeCode::I32,
                name: None
            }
        );
    }

    #[test]
    fn parse_cluster_string_i32_bool() {
        let field_str = make_named_section(0x30, &[0xFF, 0xFF, 0xFF, 0xFF], "label");
        let field_i32 = make_named_section(0x03, &[], "value");
        let field_bool = make_named_section(0x21, &[], "active");

        let mut payload = Vec::new();
        payload.extend_from_slice(&3u16.to_be_bytes()); // 3 fields
        payload.extend_from_slice(&field_str);
        payload.extend_from_slice(&field_i32);
        payload.extend_from_slice(&field_bool);

        let bytes = make_section(0x50, &payload);
        let td = parse(&bytes).unwrap();

        match td {
            TypeDescriptor::Cluster { fields, name } => {
                assert_eq!(fields.len(), 3);
                assert_eq!(name, None);
                assert_eq!(fields[0].name(), Some("label"));
                assert_eq!(fields[1].name(), Some("value"));
                assert_eq!(fields[2].name(), Some("active"));
                assert!(matches!(fields[0], TypeDescriptor::String { .. }));
                assert!(matches!(
                    fields[1],
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::I32,
                        ..
                    }
                ));
                assert!(matches!(fields[2], TypeDescriptor::Boolean { .. }));
            }
            _ => panic!("expected Cluster, got {td:?}"),
        }
    }

    #[test]
    fn parse_enum_3_members() {
        // Enum payload: member_count (u16 BE) + N pascal strings
        let mut payload = Vec::new();
        payload.extend_from_slice(&3u16.to_be_bytes()); // 3 members
                                                        // Pascal strings: len byte + chars
        payload.push(3);
        payload.extend_from_slice(b"Red");
        payload.push(5);
        payload.extend_from_slice(b"Green");
        payload.push(4);
        payload.extend_from_slice(b"Blue");

        let bytes = make_section(0x16, &payload); // EnumU16
        let td = parse(&bytes).unwrap();

        match td {
            TypeDescriptor::Enum {
                base_type,
                members,
                name,
            } => {
                assert_eq!(base_type, LvTypeCode::EnumU16);
                assert_eq!(members, vec!["Red", "Green", "Blue"]);
                assert_eq!(name, None);
            }
            _ => panic!("expected Enum, got {td:?}"),
        }
    }

    #[test]
    fn parse_physical_quantity() {
        // Physical quantity: unit_count (u16) + units
        let mut payload = Vec::new();
        payload.extend_from_slice(&2u16.to_be_bytes()); // 2 unit components
                                                        // meter^1
        payload.extend_from_slice(&3u16.to_be_bytes()); // unit code: m
        payload.extend_from_slice(&1i16.to_be_bytes()); // power: 1
                                                        // second^-2
        payload.extend_from_slice(&2u16.to_be_bytes()); // unit code: s
        payload.extend_from_slice(&(-2i16).to_be_bytes()); // power: -2

        let bytes = make_section(0x1A, &payload); // PhysDbl
        let td = parse(&bytes).unwrap();

        match td {
            TypeDescriptor::PhysicalQuantity {
                base_type,
                units,
                name,
            } => {
                assert_eq!(base_type, LvTypeCode::PhysDbl);
                assert_eq!(units.len(), 2);
                assert_eq!(units[0], PhysicalUnit { unit: 3, power: 1 });
                assert_eq!(units[1], PhysicalUnit { unit: 2, power: -2 });
                assert_eq!(name, None);
            }
            _ => panic!("expected PhysicalQuantity, got {td:?}"),
        }
    }

    #[test]
    fn parse_map_string_i32() {
        let key_section = make_section(0x30, &[0xFF, 0xFF, 0xFF, 0xFF]); // String
        let val_section = make_section(0x03, &[]); // I32

        let mut payload = Vec::new();
        payload.extend_from_slice(&2u16.to_be_bytes()); // n_types = 2
        payload.extend_from_slice(&key_section);
        payload.extend_from_slice(&val_section);

        let bytes = make_section(0x74, &payload);
        let td = parse(&bytes).unwrap();

        match td {
            TypeDescriptor::Map {
                key, value, name, ..
            } => {
                assert!(matches!(*key, TypeDescriptor::String { .. }));
                assert!(matches!(
                    *value,
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::I32,
                        ..
                    }
                ));
                assert_eq!(name, None);
            }
            _ => panic!("expected Map, got {td:?}"),
        }
    }

    #[test]
    fn parse_set_of_dbl() {
        let element_section = make_section(0x0A, &[]); // Dbl
        let bytes = make_section(0x73, &element_section);
        let td = parse(&bytes).unwrap();

        match td {
            TypeDescriptor::Set { element, name } => {
                assert_eq!(
                    *element,
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::Dbl,
                        name: None
                    }
                );
                assert_eq!(name, None);
            }
            _ => panic!("expected Set, got {td:?}"),
        }
    }

    #[test]
    fn parse_variant_type() {
        let bytes = make_section(0x53, &[]);
        let td = parse(&bytes).unwrap();
        assert!(matches!(td, TypeDescriptor::Variant { name: None }));
    }

    #[test]
    fn parse_empty_input_returns_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn parse_invalid_type_code() {
        let bytes = make_section(0xFF, &[]);
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn parse_truncated_input() {
        // Only 2 bytes — not enough for a complete section header
        assert!(parse(&[0x00, 0x08]).is_err());
    }

    #[test]
    fn parse_nested_cluster_of_arrays() {
        // Cluster { Array<I32>, Array<DBL> }
        let arr_i32_elem = make_section(0x03, &[]); // I32
        let mut arr_i32_payload = Vec::new();
        arr_i32_payload.extend_from_slice(&1u16.to_be_bytes()); // ndims=1
        arr_i32_payload.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
        arr_i32_payload.extend_from_slice(&arr_i32_elem);
        let arr_i32 = make_named_section(0x40, &arr_i32_payload, "ints");

        let arr_dbl_elem = make_section(0x0A, &[]); // DBL
        let mut arr_dbl_payload = Vec::new();
        arr_dbl_payload.extend_from_slice(&1u16.to_be_bytes());
        arr_dbl_payload.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
        arr_dbl_payload.extend_from_slice(&arr_dbl_elem);
        let arr_dbl = make_named_section(0x40, &arr_dbl_payload, "vals");

        let mut cluster_payload = Vec::new();
        cluster_payload.extend_from_slice(&2u16.to_be_bytes()); // 2 fields
        cluster_payload.extend_from_slice(&arr_i32);
        cluster_payload.extend_from_slice(&arr_dbl);

        let bytes = make_section(0x50, &cluster_payload);
        let td = parse(&bytes).unwrap();

        match &td {
            TypeDescriptor::Cluster { fields, .. } => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name(), Some("ints"));
                assert_eq!(fields[1].name(), Some("vals"));
                assert!(matches!(fields[0], TypeDescriptor::Array { ndims: 1, .. }));
            }
            _ => panic!("expected Cluster"),
        }
    }

    // ---- Little-endian tests ----

    /// Build a native-endian section (multi-byte fields in platform byte order).
    /// The flags/code pair is a single u16 (`flags << 8 | code`), so on a
    /// little-endian host the bytes are `[code][flags]`.
    fn make_section_native(code: u8, payload: &[u8]) -> Vec<u8> {
        let total_len = 4 + payload.len();
        let mut buf = Vec::with_capacity(total_len);
        buf.extend_from_slice(&(total_len as u16).to_ne_bytes());
        buf.extend_from_slice(&(code as u16).to_ne_bytes()); // flags = 0x00
        buf.extend_from_slice(payload);
        if buf.len() % 2 != 0 {
            buf.push(0x00);
            let new_len = buf.len() as u16;
            let ne = new_len.to_ne_bytes();
            buf[0] = ne[0];
            buf[1] = ne[1];
        }
        buf
    }

    #[test]
    fn parse_native_scalar_i32() {
        let bytes = make_section_native(0x03, &[]);
        let td = parse_native(&bytes).unwrap();
        assert_eq!(
            td,
            TypeDescriptor::Numeric {
                code: LvTypeCode::I32,
                name: None
            }
        );
    }

    #[test]
    fn parse_native_cluster_dbl_i32() {
        let field_dbl = make_section_native(0x0A, &[]);
        let field_i32 = make_section_native(0x03, &[]);

        let mut payload = Vec::new();
        payload.extend_from_slice(&2u16.to_ne_bytes());
        payload.extend_from_slice(&field_dbl);
        payload.extend_from_slice(&field_i32);

        let bytes = make_section_native(0x50, &payload);
        let td = parse_native(&bytes).unwrap();

        match td {
            TypeDescriptor::Cluster { fields, .. } => {
                assert_eq!(fields.len(), 2);
                assert!(matches!(
                    fields[0],
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::Dbl,
                        ..
                    }
                ));
                assert!(matches!(
                    fields[1],
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::I32,
                        ..
                    }
                ));
            }
            _ => panic!("expected Cluster, got {td:?}"),
        }
    }

    #[test]
    fn parse_native_1d_array_of_i32() {
        let element = make_section_native(0x03, &[]);
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_ne_bytes());
        payload.extend_from_slice(&0xFFFFFFFFu32.to_ne_bytes());
        payload.extend_from_slice(&element);

        let bytes = make_section_native(0x40, &payload);
        let td = parse_native(&bytes).unwrap();

        match td {
            TypeDescriptor::Array { ndims, element, .. } => {
                assert_eq!(ndims, 1);
                assert_eq!(
                    *element,
                    TypeDescriptor::Numeric {
                        code: LvTypeCode::I32,
                        name: None
                    }
                );
            }
            _ => panic!("expected Array, got {td:?}"),
        }
    }

    /// Real descriptor bytes captured from `GetTypeFromLvVariant` on
    /// LabVIEW 2025 x64: `To Variant` of an external DVR referencing a
    /// 2-D DBL array (see docs/variant-implementation-plan.md,
    /// "Verified Findings"). This is a golden vector, NOT built by the
    /// same helpers as the parser — it validates real-world layout,
    /// including the `[code][flags]` native byte order.
    #[test]
    #[cfg(target_endian = "little")]
    fn parse_native_real_edvr_descriptor() {
        #[rustfmt::skip]
        let bytes: [u8; 42] = [
            0x2A, 0x00,             // len = 42 (LE)
            0x70, 0x40,             // code 0x70 refnum, flags 0x40 has-name
            0x20, 0x00,             // refnum kind 0x0020
            0x1A, 0x00,             // nested section len = 26
            0x40, 0x00,             //   code 0x40 array, flags 0
            0x02, 0x00,             //   ndims = 2
            0xFF, 0xFF, 0xFF, 0xFF, //   dim placeholder
            0xFF, 0xFF, 0xFF, 0xFF, //   dim placeholder
            0x0C, 0x00,             //   element section len = 12
            0x0A, 0x40,             //     code 0x0A DBL, flags 0x40 has-name
            0x07, b'N', b'u', b'm', b'e', b'r', b'i', b'c',
            0x08, b'E', b'D', b'V', b'R', b'_', b'R', b'e', b'f',
            0x00,                   // pad to even length
        ];
        let td = parse_native(&bytes).unwrap();
        match td {
            TypeDescriptor::Refnum {
                kind,
                referenced: Some(referenced),
                name,
            } => {
                assert_eq!(kind, 0x0020);
                assert_eq!(name.as_deref(), Some("EDVR_Ref"));
                match *referenced {
                    TypeDescriptor::Array { ndims, element, .. } => {
                        assert_eq!(ndims, 2);
                        assert_eq!(
                            *element,
                            TypeDescriptor::Numeric {
                                code: LvTypeCode::Dbl,
                                name: Some("Numeric".to_string()),
                            }
                        );
                    }
                    other => panic!("expected Array, got {other:?}"),
                }
            }
            other => panic!("expected Refnum with referenced type, got {other:?}"),
        }
    }

    #[test]
    fn parse_refnum_without_referenced_type() {
        // A refnum carrying only the kind word and a name — no nested type.
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x0001u16.to_be_bytes());
        let bytes = make_named_section(0x70, &payload, "queue");
        let td = parse(&bytes).unwrap();
        assert_eq!(
            td,
            TypeDescriptor::Refnum {
                kind: 1,
                referenced: None,
                name: Some("queue".to_string()),
            }
        );
    }
}
