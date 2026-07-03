//! Dynamic value reading for LabVIEW data guided by a [`TypeDescriptor`].
//!
//! [`read_value`] walks a parsed type descriptor and the memory layout rules
//! from [`super::layout`] to read *any* supported LabVIEW value — scalars,
//! strings, N-dimensional arrays, clusters, nested clusters, timestamps,
//! enums and refnums — into an [`LvValue`] tree. It is the runtime-dynamic
//! equivalent of LabVIEW's "Variant To Data" and is the core of the
//! integration test oracle: [`LvValue`]'s [`Display`](std::fmt::Display)
//! implementation renders a *canonical string* which LabVIEW test VIs
//! compare against hand-authored expected strings.
//!
//! This module performs pure pointer-walking. It needs no LabVIEW runtime
//! functions — handles are dereferenced directly in memory — so it is fully
//! unit-testable against fabricated buffers.
//!
//! # Canonical string format
//!
//! The rendering is deterministic so expected strings can be authored in
//! LabVIEW test VIs:
//!
//! | Value | Rendering | Example |
//! |-------|-----------|---------|
//! | Integer | plain decimal | `42`, `-7` |
//! | Float | Rust `Display` (shortest round-trip, no exponent) | `3.5`, `-0.25`, `NaN`, `inf` |
//! | Boolean | `true` / `false` | `true` |
//! | String | double-quoted; printable ASCII verbatim, `"` and `\` escaped, all other bytes as `\xNN` (lowercase hex) | `"abc"`, `"a\x0ab"` |
//! | Timestamp | `ts(<seconds>,<fractions>)` | `ts(3856147200,0)` |
//! | Enum | `enum(<value>:<member>)`, or `enum(<value>)` if out of range | `enum(2:Resistance)` |
//! | Refnum | `refnum(0x<cookie, 8 hex digits>)` | `refnum(0x12300000)` |
//! | 1-D array | `[e0,e1,…]` | `[1,2,3]` |
//! | N-D array | `dims[d0,d1,…]:[flat elements…]` (row-major) | `dims[2,2]:[1,2,3,4]` |
//! | Cluster | `{name=value,…}`; `name=` omitted when the field is unnamed | `{label="a",value=42}` |
//! | Void | `void` | `void` |
//! | Unsupported | `unsupported(<type name>)` | `unsupported(Waveform)` |
//!
//! Float caveat: prefer test values with a short exact decimal representation
//! (`3.5`, `-0.25`); Rust's `Display` never uses scientific notation, so very
//! large magnitudes render with all digits.

use std::ffi::c_void;
use std::fmt;

use crate::errors::{InternalError, LVInteropError};

use super::layout::align_to;
use super::types::{LvTypeCode, TypeDescriptor};

/// A dynamically-typed LabVIEW value read via [`read_value`].
#[derive(Debug, Clone, PartialEq)]
pub enum LvValue {
    /// Void / empty variant — no data.
    Void,
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    Bool(bool),
    /// Raw string bytes (LabVIEW strings are byte strings, not UTF-8).
    String(Vec<u8>),
    /// LabVIEW timestamp: seconds since 1904-01-01 UTC + 2^-64 fractions.
    Timestamp { seconds: i64, fractions: u64 },
    /// Enum value with the resolved member name where in range.
    Enum { value: u32, member: Option<String> },
    /// Refnum magic cookie. Note the cookie value varies between runs.
    Refnum { cookie: u32 },
    /// N-dimensional array; elements in row-major order.
    Array { dims: Vec<usize>, elements: Vec<LvValue> },
    /// Cluster fields in order, with names where the descriptor carries them.
    Cluster(Vec<(Option<String>, LvValue)>),
    /// Type is parseable but not yet readable (complex, waveform, map, set,
    /// path, variant-in-variant, extended float, physical quantity).
    Unsupported(LvTypeCode),
}

/// Read a LabVIEW value of type `td` from `ptr`.
///
/// `ptr` must be the address where the value *sits*: for scalar/inline types
/// this is the data itself; for handle types (strings, arrays) it is the
/// address of the handle pointer. This matches what `LvVariantGetDataPtr`
/// returns for a top-level value and what cluster field offsets produce for
/// embedded values, so the function recurses uniformly.
///
/// Null handles are read as empty strings/arrays (matching LabVIEW's
/// canonical empty representation) rather than errors.
///
/// # Safety
///
/// - `ptr` must point to live memory laid out as described by `td` for the
///   current platform. A mismatched descriptor reads garbage or faults.
/// - Handle pointers reached through the data must be valid or null.
/// - All reads are unaligned-safe (`read_unaligned`), so packed 32-bit
///   layouts are fine.
///
/// # Errors
///
/// - [`InternalError::BrokenVariant`] on negative string lengths or array
///   dimensions (corrupt data or a wrong descriptor).
pub unsafe fn read_value(
    td: &TypeDescriptor,
    ptr: *const c_void,
) -> Result<LvValue, LVInteropError> {
    match td {
        TypeDescriptor::Void => Ok(LvValue::Void),

        TypeDescriptor::Numeric { code, .. } => Ok(read_numeric(*code, ptr)),

        TypeDescriptor::Boolean { .. } => {
            Ok(LvValue::Bool(std::ptr::read_unaligned(ptr as *const u8) != 0))
        }

        TypeDescriptor::String { .. } => read_string(ptr),

        TypeDescriptor::Enum {
            base_type, members, ..
        } => {
            let value = match base_type {
                LvTypeCode::EnumU8 => std::ptr::read_unaligned(ptr as *const u8) as u32,
                LvTypeCode::EnumU16 => std::ptr::read_unaligned(ptr as *const u16) as u32,
                _ => std::ptr::read_unaligned(ptr as *const u32),
            };
            let member = members.get(value as usize).cloned();
            Ok(LvValue::Enum { value, member })
        }

        TypeDescriptor::Timestamp { .. } => {
            // LVTime in memory: i64 seconds then u64 fractions (host-native).
            let seconds = std::ptr::read_unaligned(ptr as *const i64);
            let fractions = std::ptr::read_unaligned((ptr as *const u8).add(8) as *const u64);
            Ok(LvValue::Timestamp { seconds, fractions })
        }

        TypeDescriptor::Refnum { .. } => {
            // The runtime data of a refnum is its 4-byte magic cookie.
            let cookie = std::ptr::read_unaligned(ptr as *const u32);
            Ok(LvValue::Refnum { cookie })
        }

        TypeDescriptor::Array { ndims, element, .. } => read_array(*ndims, element, ptr),

        TypeDescriptor::Cluster { fields, .. } => {
            let base = ptr as *const u8;
            let mut offset = 0usize;
            let mut items = Vec::with_capacity(fields.len());
            for field in fields {
                // Must accumulate exactly like `TypeDescriptor::offset_of`.
                offset = align_to(offset, field.alignment());
                let value = read_value(field, base.add(offset) as *const c_void)?;
                items.push((field.name().map(str::to_string), value));
                offset += field.size();
            }
            Ok(LvValue::Cluster(items))
        }

        // Parseable but not yet readable.
        other => Ok(LvValue::Unsupported(other.type_code())),
    }
}

/// Read a scalar numeric. Extended floats and complex types are not yet
/// supported and return [`LvValue::Unsupported`].
unsafe fn read_numeric(code: LvTypeCode, ptr: *const c_void) -> LvValue {
    match code {
        LvTypeCode::I8 => LvValue::I8(std::ptr::read_unaligned(ptr as *const i8)),
        LvTypeCode::I16 => LvValue::I16(std::ptr::read_unaligned(ptr as *const i16)),
        LvTypeCode::I32 => LvValue::I32(std::ptr::read_unaligned(ptr as *const i32)),
        LvTypeCode::I64 => LvValue::I64(std::ptr::read_unaligned(ptr as *const i64)),
        LvTypeCode::U8 => LvValue::U8(std::ptr::read_unaligned(ptr as *const u8)),
        LvTypeCode::U16 => LvValue::U16(std::ptr::read_unaligned(ptr as *const u16)),
        LvTypeCode::U32 => LvValue::U32(std::ptr::read_unaligned(ptr as *const u32)),
        LvTypeCode::U64 => LvValue::U64(std::ptr::read_unaligned(ptr as *const u64)),
        LvTypeCode::Sgl => LvValue::F32(std::ptr::read_unaligned(ptr as *const f32)),
        LvTypeCode::Dbl => LvValue::F64(std::ptr::read_unaligned(ptr as *const f64)),
        other => LvValue::Unsupported(other),
    }
}

/// Read a string. `ptr` is the address of an `LStr` handle
/// (`*mut *mut LStr`). Null handle or null inner pointer reads as empty.
unsafe fn read_string(ptr: *const c_void) -> Result<LvValue, LVInteropError> {
    let handle = std::ptr::read_unaligned(ptr as *const *mut *mut u8);
    if handle.is_null() {
        return Ok(LvValue::String(Vec::new()));
    }
    let lstr = *handle;
    if lstr.is_null() {
        return Ok(LvValue::String(Vec::new()));
    }
    // LStr layout: [i32 length][bytes…]
    let len = std::ptr::read_unaligned(lstr as *const i32);
    if len < 0 {
        return Err(InternalError::BrokenVariant.into());
    }
    let data = lstr.add(std::mem::size_of::<i32>());
    let bytes = std::slice::from_raw_parts(data, len as usize).to_vec();
    Ok(LvValue::String(bytes))
}

/// Read an N-dimensional array. `ptr` is the address of the array handle.
///
/// Array data layout (h5labview `readwrite.c`):
/// `[ndims × i32 dimension sizes][padding to element alignment][elements…]`
unsafe fn read_array(
    ndims: u16,
    element: &TypeDescriptor,
    ptr: *const c_void,
) -> Result<LvValue, LVInteropError> {
    let ndims = ndims as usize;

    let handle = std::ptr::read_unaligned(ptr as *const *mut *mut u8);
    if handle.is_null() {
        return Ok(LvValue::Array {
            dims: vec![0; ndims],
            elements: Vec::new(),
        });
    }
    let data = *handle;
    if data.is_null() {
        return Ok(LvValue::Array {
            dims: vec![0; ndims],
            elements: Vec::new(),
        });
    }

    let mut dims = Vec::with_capacity(ndims);
    let mut count = 1usize;
    for i in 0..ndims {
        let dim = std::ptr::read_unaligned((data as *const u8).add(4 * i) as *const i32);
        if dim < 0 {
            return Err(InternalError::BrokenVariant.into());
        }
        count = count
            .checked_mul(dim as usize)
            .ok_or(InternalError::BrokenVariant)?;
        dims.push(dim as usize);
    }

    let elem_align = element.alignment();
    let elem_size = element.size();
    let stride = align_to(elem_size, elem_align);
    let start = align_to(4 * ndims, elem_align);

    let mut elements = Vec::with_capacity(count);
    for i in 0..count {
        let elem_ptr = (data as *const u8).add(start + i * stride) as *const c_void;
        elements.push(read_value(element, elem_ptr)?);
    }

    Ok(LvValue::Array { dims, elements })
}

impl fmt::Display for LvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LvValue::Void => write!(f, "void"),
            LvValue::I8(v) => write!(f, "{v}"),
            LvValue::I16(v) => write!(f, "{v}"),
            LvValue::I32(v) => write!(f, "{v}"),
            LvValue::I64(v) => write!(f, "{v}"),
            LvValue::U8(v) => write!(f, "{v}"),
            LvValue::U16(v) => write!(f, "{v}"),
            LvValue::U32(v) => write!(f, "{v}"),
            LvValue::U64(v) => write!(f, "{v}"),
            LvValue::F32(v) => write!(f, "{v}"),
            LvValue::F64(v) => write!(f, "{v}"),
            LvValue::Bool(v) => write!(f, "{v}"),
            LvValue::String(bytes) => {
                write!(f, "\"")?;
                for &b in bytes {
                    match b {
                        b'"' => write!(f, "\\\"")?,
                        b'\\' => write!(f, "\\\\")?,
                        0x20..=0x7E => write!(f, "{}", b as char)?,
                        other => write!(f, "\\x{other:02x}")?,
                    }
                }
                write!(f, "\"")
            }
            LvValue::Timestamp { seconds, fractions } => {
                write!(f, "ts({seconds},{fractions})")
            }
            LvValue::Enum { value, member } => match member {
                Some(member) => write!(f, "enum({value}:{member})"),
                None => write!(f, "enum({value})"),
            },
            LvValue::Refnum { cookie } => write!(f, "refnum(0x{cookie:08x})"),
            LvValue::Array { dims, elements } => {
                if dims.len() > 1 {
                    write!(f, "dims[")?;
                    for (i, d) in dims.iter().enumerate() {
                        if i > 0 {
                            write!(f, ",")?;
                        }
                        write!(f, "{d}")?;
                    }
                    write!(f, "]:")?;
                }
                write!(f, "[")?;
                for (i, e) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, "]")
            }
            LvValue::Cluster(fields) => {
                write!(f, "{{")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    if let Some(name) = name {
                        write!(f, "{name}=")?;
                    }
                    write!(f, "{value}")?;
                }
                write!(f, "}}")
            }
            LvValue::Unsupported(code) => write!(f, "unsupported({})", code.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(code: LvTypeCode) -> TypeDescriptor {
        TypeDescriptor::Numeric { code, name: None }
    }

    fn named_num(code: LvTypeCode, name: &str) -> TypeDescriptor {
        TypeDescriptor::Numeric {
            code,
            name: Some(name.to_string()),
        }
    }

    #[test]
    fn read_scalar_i32() {
        let value = 42i32;
        let td = num(LvTypeCode::I32);
        let read = unsafe { read_value(&td, &value as *const i32 as *const c_void) }.unwrap();
        assert_eq!(read, LvValue::I32(42));
        assert_eq!(read.to_string(), "42");
    }

    #[test]
    fn read_scalar_f64() {
        let value = -0.25f64;
        let td = num(LvTypeCode::Dbl);
        let read = unsafe { read_value(&td, &value as *const f64 as *const c_void) }.unwrap();
        assert_eq!(read, LvValue::F64(-0.25));
        assert_eq!(read.to_string(), "-0.25");
    }

    #[test]
    fn read_bool() {
        let value = 1u8;
        let td = TypeDescriptor::Boolean { name: None };
        let read = unsafe { read_value(&td, &value as *const u8 as *const c_void) }.unwrap();
        assert_eq!(read, LvValue::Bool(true));
        assert_eq!(read.to_string(), "true");
    }

    #[test]
    fn read_timestamp() {
        // LVTime memory: i64 seconds then u64 fractions.
        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&3_856_147_200i64.to_ne_bytes());
        buf[8..].copy_from_slice(&7u64.to_ne_bytes());
        let td = TypeDescriptor::Timestamp { name: None };
        let read = unsafe { read_value(&td, buf.as_ptr() as *const c_void) }.unwrap();
        assert_eq!(
            read,
            LvValue::Timestamp {
                seconds: 3_856_147_200,
                fractions: 7
            }
        );
        assert_eq!(read.to_string(), "ts(3856147200,7)");
    }

    #[test]
    fn read_enum_u16() {
        let value = 2u16;
        let td = TypeDescriptor::Enum {
            base_type: LvTypeCode::EnumU16,
            members: vec!["Voltage".into(), "Current".into(), "Resistance".into()],
            name: None,
        };
        let read = unsafe { read_value(&td, &value as *const u16 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "enum(2:Resistance)");
    }

    #[test]
    fn read_enum_out_of_range() {
        let value = 9u8;
        let td = TypeDescriptor::Enum {
            base_type: LvTypeCode::EnumU8,
            members: vec!["A".into()],
            name: None,
        };
        let read = unsafe { read_value(&td, &value as *const u8 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "enum(9)");
    }

    #[test]
    fn read_refnum_cookie() {
        let cookie = 0x1230_0000u32;
        let td = TypeDescriptor::Refnum {
            kind: 0x20,
            referenced: None,
            name: None,
        };
        let read = unsafe { read_value(&td, &cookie as *const u32 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "refnum(0x12300000)");
    }

    /// Build a fake LStr buffer: [i32 len][bytes...]. Returns the buffer.
    fn fake_lstr(content: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + content.len());
        buf.extend_from_slice(&(content.len() as i32).to_ne_bytes());
        buf.extend_from_slice(content);
        buf
    }

    #[test]
    fn read_string_through_handle_chain() {
        let lstr = fake_lstr(b"hello \"world\"\n");
        let mut inner: *mut u8 = lstr.as_ptr() as *mut u8;
        let handle: *mut *mut u8 = &mut inner;
        let td = TypeDescriptor::String { name: None };
        // `ptr` is the address where the handle sits.
        let read =
            unsafe { read_value(&td, &handle as *const *mut *mut u8 as *const c_void) }.unwrap();
        assert_eq!(read, LvValue::String(b"hello \"world\"\n".to_vec()));
        assert_eq!(read.to_string(), "\"hello \\\"world\\\"\\x0a\"");
    }

    #[test]
    fn read_null_string_handle_as_empty() {
        let handle: *mut *mut u8 = std::ptr::null_mut();
        let td = TypeDescriptor::String { name: None };
        let read =
            unsafe { read_value(&td, &handle as *const *mut *mut u8 as *const c_void) }.unwrap();
        assert_eq!(read, LvValue::String(Vec::new()));
        assert_eq!(read.to_string(), "\"\"");
    }

    #[test]
    fn read_negative_string_length_is_error() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(-1i32).to_ne_bytes());
        let mut inner: *mut u8 = buf.as_ptr() as *mut u8;
        let handle: *mut *mut u8 = &mut inner;
        let td = TypeDescriptor::String { name: None };
        let read = unsafe { read_value(&td, &handle as *const *mut *mut u8 as *const c_void) };
        assert!(read.is_err());
    }

    /// Build a fake 1-D array buffer for the given element bytes, matching
    /// the reader's layout rules ([i32 dim][pad to elem alignment][data]).
    fn fake_1d_array(dim: i32, elem_align: usize, elem_bytes: &[u8]) -> Vec<u8> {
        let start = align_to(4, elem_align);
        let mut buf = vec![0u8; start + elem_bytes.len()];
        buf[..4].copy_from_slice(&dim.to_ne_bytes());
        buf[start..].copy_from_slice(elem_bytes);
        buf
    }

    #[test]
    fn read_1d_array_of_i32() {
        let mut elems = Vec::new();
        for v in [10i32, -20, 30] {
            elems.extend_from_slice(&v.to_ne_bytes());
        }
        let td = TypeDescriptor::Array {
            ndims: 1,
            element: Box::new(num(LvTypeCode::I32)),
            name: None,
        };
        let elem_align = num(LvTypeCode::I32).alignment();
        let buf = fake_1d_array(3, elem_align, &elems);
        let mut inner: *mut u8 = buf.as_ptr() as *mut u8;
        let handle: *mut *mut u8 = &mut inner;
        let read =
            unsafe { read_value(&td, &handle as *const *mut *mut u8 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "[10,-20,30]");
    }

    #[test]
    fn read_null_array_handle_as_empty() {
        let handle: *mut *mut u8 = std::ptr::null_mut();
        let td = TypeDescriptor::Array {
            ndims: 2,
            element: Box::new(num(LvTypeCode::Dbl)),
            name: None,
        };
        let read =
            unsafe { read_value(&td, &handle as *const *mut *mut u8 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "dims[0,0]:[]");
    }

    #[test]
    fn read_2d_array_of_i32() {
        // dims [2,3], elements 1..=6 row-major.
        let elem_align = num(LvTypeCode::I32).alignment();
        let start = align_to(8, elem_align);
        let mut buf = vec![0u8; start + 6 * 4];
        buf[..4].copy_from_slice(&2i32.to_ne_bytes());
        buf[4..8].copy_from_slice(&3i32.to_ne_bytes());
        for (i, v) in (1i32..=6).enumerate() {
            buf[start + i * 4..start + i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
        let td = TypeDescriptor::Array {
            ndims: 2,
            element: Box::new(num(LvTypeCode::I32)),
            name: None,
        };
        let mut inner: *mut u8 = buf.as_ptr() as *mut u8;
        let handle: *mut *mut u8 = &mut inner;
        let read =
            unsafe { read_value(&td, &handle as *const *mut *mut u8 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "dims[2,3]:[1,2,3,4,5,6]");
    }

    #[test]
    fn read_cluster_with_padding() {
        // Cluster {a: u8, b: i32} — buffer built with the same layout rules
        // the reader uses (offset checked against `offset_of`).
        let td = TypeDescriptor::Cluster {
            fields: vec![
                named_num(LvTypeCode::U8, "a"),
                named_num(LvTypeCode::I32, "b"),
            ],
            name: None,
        };
        let b_offset = td.offset_of(1).unwrap();
        let mut buf = vec![0u8; td.size()];
        buf[0] = 7;
        buf[b_offset..b_offset + 4].copy_from_slice(&1234i32.to_ne_bytes());
        let read = unsafe { read_value(&td, buf.as_ptr() as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "{a=7,b=1234}");
    }

    #[test]
    fn read_nested_cluster() {
        // {a: u8, inner: {x: u8, y: u32}, b: u8}
        let inner_td = TypeDescriptor::Cluster {
            fields: vec![
                named_num(LvTypeCode::U8, "x"),
                named_num(LvTypeCode::U32, "y"),
            ],
            name: Some("inner".to_string()),
        };
        let td = TypeDescriptor::Cluster {
            fields: vec![
                named_num(LvTypeCode::U8, "a"),
                inner_td.clone(),
                named_num(LvTypeCode::U8, "b"),
            ],
            name: None,
        };
        let inner_offset = td.offset_of(1).unwrap();
        let y_offset = inner_td.offset_of(1).unwrap();
        let b_offset = td.offset_of(2).unwrap();

        let mut buf = vec![0u8; td.size()];
        buf[0] = 1;
        buf[inner_offset] = 2;
        buf[inner_offset + y_offset..inner_offset + y_offset + 4]
            .copy_from_slice(&300u32.to_ne_bytes());
        buf[b_offset] = 4;

        let read = unsafe { read_value(&td, buf.as_ptr() as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "{a=1,inner={x=2,y=300},b=4}");
    }

    #[test]
    fn read_cluster_with_string_field() {
        // {flag: bool, name: string} — the string field holds a handle.
        let td = TypeDescriptor::Cluster {
            fields: vec![
                TypeDescriptor::Boolean {
                    name: Some("flag".to_string()),
                },
                TypeDescriptor::String {
                    name: Some("name".to_string()),
                },
            ],
            name: None,
        };
        let lstr = fake_lstr(b"abc");
        let mut inner: *mut u8 = lstr.as_ptr() as *mut u8;
        let handle: *mut *mut u8 = &mut inner;

        let str_offset = td.offset_of(1).unwrap();
        let mut buf = vec![0u8; td.size()];
        buf[0] = 1;
        let handle_bytes = (handle as usize).to_ne_bytes();
        buf[str_offset..str_offset + std::mem::size_of::<usize>()]
            .copy_from_slice(&handle_bytes);

        let read = unsafe { read_value(&td, buf.as_ptr() as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "{flag=true,name=\"abc\"}");
    }

    #[test]
    fn unsupported_types_render_as_unsupported() {
        let value = 0u128; // enough zeroed memory for any scalar read
        let td = num(LvTypeCode::CDbl);
        let read = unsafe { read_value(&td, &value as *const u128 as *const c_void) }.unwrap();
        assert_eq!(read.to_string(), "unsupported(CDB)");
    }

    #[test]
    fn void_renders_as_void() {
        let td = TypeDescriptor::Void;
        let read = unsafe { read_value(&td, std::ptr::null()) }.unwrap();
        assert_eq!(read.to_string(), "void");
    }
}
