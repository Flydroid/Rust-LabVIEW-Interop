//! Memory layout computation for LabVIEW type descriptors.
//!
//! Computes in-memory size, alignment, and field offsets for LabVIEW data types.
//! These match LabVIEW's actual memory layout, which varies by platform:
//!
//! - **Windows 64-bit**: Standard C alignment (max 8 bytes)
//! - **Windows 32-bit**: Packed (no padding, alignment 1)
//! - **Linux**: Alignment capped at 4 bytes
//!
//! Reference: h5labview `h5labview.h` LVALIGNMENT/DO_ALIGN macros

use super::types::{LvTypeCode, TypeDescriptor};

/// Returns the LabVIEW maximum memory alignment for the current platform.
///
/// - Windows 64-bit: 8
/// - Windows 32-bit: 0 (packed, no alignment)
/// - Linux: 4
pub const fn platform_alignment() -> usize {
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    {
        8
    }
    #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
    {
        0 // packed — no alignment
    }
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    {
        4
    }
    #[cfg(all(target_os = "linux", target_pointer_width = "32"))]
    {
        4
    }
    // macOS: same as linux
    #[cfg(target_os = "macos")]
    {
        4
    }
}

/// Align `offset` up to the given `alignment`.
/// If `alignment` is 0 or 1, returns `offset` unchanged.
pub const fn align_to(offset: usize, alignment: usize) -> usize {
    if alignment <= 1 {
        return offset;
    }
    (offset + alignment - 1) & !(alignment - 1)
}

/// The size of LabVIEW's Extended float type.
/// On Windows (MSVC), this is typically 16 bytes.
/// On other platforms, it may differ.
const LV_EXT_SIZE: usize = {
    #[cfg(target_os = "windows")]
    {
        16 // LabVIEW uses 128-bit extended on Windows
    }
    #[cfg(not(target_os = "windows"))]
    {
        16 // Assume 16 for now, may vary
    }
};

/// Cap an alignment value to the platform maximum.
/// On Win32 (packed), this returns 1 regardless of input.
fn capped_alignment(align: usize) -> usize {
    let max = platform_alignment();
    if max == 0 {
        1 // packed
    } else if align > max {
        max
    } else if align == 0 {
        1
    } else {
        align
    }
}

/// Waveform subcode-to-type-code mapping.
/// Reference: h5labview `typeconv.c` `waveform_to_tc` function.
fn waveform_numeric_size(subcode: u8) -> usize {
    match subcode {
        0x14 => 1,  // I8
        0x11 => 1,  // U8
        0x02 => 2,  // I16
        0x12 => 2,  // U16
        0x15 => 4,  // I32
        0x13 => 4,  // U32
        0x19 => 8,  // I64
        0x20 => 8,  // U64
        0x05 => 4,  // SGL
        0x03 => 8,  // DBL
        _ => 8,     // Default to DBL
    }
}

impl TypeDescriptor {
    /// Returns the in-memory size of this type in bytes.
    ///
    /// For handle types (String, Array, Path, Variant, Map, Set), returns pointer size.
    /// For clusters, includes padding per platform rules.
    pub fn size(&self) -> usize {
        match self {
            TypeDescriptor::Numeric { code, .. } => numeric_size(*code),
            TypeDescriptor::Boolean { .. } => 1,
            TypeDescriptor::Enum { base_type, .. } => enum_size(*base_type),
            TypeDescriptor::PhysicalQuantity { base_type, .. } => {
                // Physical quantities have the same size as their underlying numeric type
                base_type
                    .physical_to_numeric()
                    .map(numeric_size)
                    .unwrap_or(8)
            }
            // Handle types: pointer-to-pointer
            TypeDescriptor::String { .. }
            | TypeDescriptor::Path { .. }
            | TypeDescriptor::Variant { .. }
            | TypeDescriptor::Map { .. }
            | TypeDescriptor::Set { .. } => std::mem::size_of::<usize>(), // pointer size

            TypeDescriptor::Array { .. } => std::mem::size_of::<usize>(), // handle = pointer

            TypeDescriptor::Cluster { fields, .. } => {
                if fields.is_empty() {
                    return 0;
                }
                let mut offset = 0usize;
                let mut max_align = 1usize;
                for field in fields {
                    let field_align = field.alignment();
                    if field_align > max_align {
                        max_align = field_align;
                    }
                    offset = align_to(offset, field_align);
                    offset += field.size();
                }
                // Tail padding to max alignment
                align_to(offset, max_align)
            }

            TypeDescriptor::Waveform { subcode, .. } => {
                // Waveform: t0 (16 bytes) + dt (8 bytes) + data handle + padding + attributes variant + padding
                // This is complex; for layout purposes the Waveform struct is defined in types/mod.rs
                // with platform-specific padding. We return its computed size.
                let timestamp_size = 16; // LVTime: i64 + u64
                let dt_size = 8; // f64
                let handle_size = std::mem::size_of::<usize>(); // array handle
                let variant_size = std::mem::size_of::<usize>(); // variant handle

                // Approximate — exact padding depends on platform
                let _ = subcode;
                timestamp_size + dt_size + handle_size + variant_size + handle_size * 2
                // Note: exact size should match the Waveform struct in types/mod.rs
            }

            TypeDescriptor::Timestamp { .. } => 16, // i64 seconds + u64 fractional

            // Refnum runtime data is a 4-byte magic cookie (verified for an
            // external DVR inside a Variant — the cookie sits at data offset 0).
            TypeDescriptor::Refnum { .. } => 4,

            TypeDescriptor::Void => 0,
        }
    }

    /// Returns the required memory alignment for this type.
    ///
    /// Alignment follows LabVIEW rules (capped at platform_alignment):
    /// - Numerics: alignment = type size
    /// - Complex: alignment = half the compound size
    /// - Extended: special handling based on floatExt size
    /// - Handle types (String, Array): pointer alignment
    /// - Clusters: max alignment of any field
    pub fn alignment(&self) -> usize {
        match self {
            TypeDescriptor::Numeric { code, .. } => numeric_alignment(*code),
            TypeDescriptor::Boolean { .. } => 1,
            TypeDescriptor::Enum { base_type, .. } => capped_alignment(enum_size(*base_type)),
            TypeDescriptor::PhysicalQuantity { base_type, .. } => base_type
                .physical_to_numeric()
                .map(numeric_alignment)
                .unwrap_or(capped_alignment(8)),

            // Handle types align to pointer size
            TypeDescriptor::String { .. }
            | TypeDescriptor::Path { .. }
            | TypeDescriptor::Array { .. }
            | TypeDescriptor::Variant { .. }
            | TypeDescriptor::Map { .. }
            | TypeDescriptor::Set { .. } => capped_alignment(std::mem::size_of::<usize>()),

            TypeDescriptor::Cluster { fields, .. } => {
                // Cluster alignment = max alignment of any field
                let max = fields.iter().map(|f| f.alignment()).max().unwrap_or(1);
                capped_alignment(max)
            }

            TypeDescriptor::Waveform { .. } => {
                // Waveform is a cluster-like type; alignment follows the largest field
                // t0 is LVTime (i64+u64 → align to 8 on Win64, but h5labview says align to 4 for timestamp)
                capped_alignment(8)
            }

            TypeDescriptor::Timestamp { .. } => {
                // h5labview: align to U32 (4) not U64
                capped_alignment(4)
            }

            // Magic cookie: u32
            TypeDescriptor::Refnum { .. } => capped_alignment(4),

            TypeDescriptor::Void => 1,
        }
    }

    /// For clusters: returns the byte offset of the field at `field_index`,
    /// accounting for platform-specific padding.
    ///
    /// Returns `None` if this is not a Cluster or if `field_index` is out of range.
    pub fn offset_of(&self, field_index: usize) -> Option<usize> {
        match self {
            TypeDescriptor::Cluster { fields, .. } => {
                if field_index >= fields.len() {
                    return None;
                }
                let mut offset = 0usize;
                for (i, field) in fields.iter().enumerate() {
                    let field_align = field.alignment();
                    offset = align_to(offset, field_align);
                    if i == field_index {
                        return Some(offset);
                    }
                    offset += field.size();
                }
                None
            }
            _ => None,
        }
    }
}

/// Returns the size of a numeric type code.
fn numeric_size(code: LvTypeCode) -> usize {
    match code {
        LvTypeCode::I8 | LvTypeCode::U8 => 1,
        LvTypeCode::I16 | LvTypeCode::U16 => 2,
        LvTypeCode::I32 | LvTypeCode::U32 => 4,
        LvTypeCode::I64 | LvTypeCode::U64 => 8,
        LvTypeCode::Sgl => 4,
        LvTypeCode::Dbl => 8,
        LvTypeCode::Ext => LV_EXT_SIZE,
        LvTypeCode::CSgl => 8,  // 2 × SGL
        LvTypeCode::CDbl => 16, // 2 × DBL
        LvTypeCode::CExt => 2 * LV_EXT_SIZE,
        _ => 0,
    }
}

/// Returns the alignment for a numeric type code (pre-capped to platform max).
fn numeric_alignment(code: LvTypeCode) -> usize {
    match code {
        // Extended float: special rule from h5labview
        // "if sizeof(floatExt) % 4 != 0 → align 1, else align to sizeof(floatExt)"
        LvTypeCode::Ext => {
            if LV_EXT_SIZE % 4 != 0 {
                1
            } else {
                capped_alignment(LV_EXT_SIZE)
            }
        }
        // Complex extended: same rule
        LvTypeCode::CExt => {
            if LV_EXT_SIZE % 4 != 0 {
                1
            } else {
                capped_alignment(LV_EXT_SIZE)
            }
        }
        // Complex types: alignment = half compound size
        LvTypeCode::CSgl => capped_alignment(4), // 8/2 = 4
        LvTypeCode::CDbl => capped_alignment(8), // 16/2 = 8
        // All other numerics: alignment = size
        _ => capped_alignment(numeric_size(code)),
    }
}

/// Returns the size of an enum's storage type.
fn enum_size(base_type: LvTypeCode) -> usize {
    match base_type {
        LvTypeCode::EnumU8 => 1,
        LvTypeCode::EnumU16 => 2,
        LvTypeCode::EnumU32 => 4,
        _ => 2, // default to U16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_i32() {
        let td = TypeDescriptor::Numeric {
            code: LvTypeCode::I32,
            name: None,
        };
        assert_eq!(td.size(), 4);
    }

    #[test]
    fn alignment_i32() {
        let td = TypeDescriptor::Numeric {
            code: LvTypeCode::I32,
            name: None,
        };
        // On Win64: alignment = 4, on Win32: 1
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        assert_eq!(td.alignment(), 4);
        #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
        assert_eq!(td.alignment(), 1);
    }

    #[test]
    fn size_dbl() {
        let td = TypeDescriptor::Numeric {
            code: LvTypeCode::Dbl,
            name: None,
        };
        assert_eq!(td.size(), 8);
    }

    #[test]
    fn size_boolean() {
        let td = TypeDescriptor::Boolean { name: None };
        assert_eq!(td.size(), 1);
    }

    #[test]
    fn size_string_is_pointer() {
        let td = TypeDescriptor::String { name: None };
        assert_eq!(td.size(), std::mem::size_of::<usize>());
    }

    #[test]
    fn size_array_is_pointer() {
        let td = TypeDescriptor::Array {
            ndims: 1,
            element: Box::new(TypeDescriptor::Numeric {
                code: LvTypeCode::Dbl,
                name: None,
            }),
            name: None,
        };
        assert_eq!(td.size(), std::mem::size_of::<usize>());
    }

    #[test]
    fn cluster_u8_u32_win64() {
        let td = TypeDescriptor::Cluster {
            fields: vec![
                TypeDescriptor::Numeric {
                    code: LvTypeCode::U8,
                    name: None,
                },
                TypeDescriptor::Numeric {
                    code: LvTypeCode::U32,
                    name: None,
                },
            ],
            name: None,
        };

        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        {
            // U8 at offset 0, U32 at offset 4 (padded), total = 8 (tail padding to align 4)
            assert_eq!(td.offset_of(0), Some(0));
            assert_eq!(td.offset_of(1), Some(4));
            assert_eq!(td.size(), 8);
        }

        #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
        {
            // Packed: U8 at offset 0, U32 at offset 1, total = 5
            assert_eq!(td.offset_of(0), Some(0));
            assert_eq!(td.offset_of(1), Some(1));
            assert_eq!(td.size(), 5);
        }
    }

    #[test]
    fn cluster_alignment_is_max_field() {
        let td = TypeDescriptor::Cluster {
            fields: vec![
                TypeDescriptor::Numeric {
                    code: LvTypeCode::U8,
                    name: None,
                },
                TypeDescriptor::Numeric {
                    code: LvTypeCode::Dbl,
                    name: None,
                },
            ],
            name: None,
        };

        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        assert_eq!(td.alignment(), 8);

        #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
        assert_eq!(td.alignment(), 1);
    }

    #[test]
    fn complex_sgl_alignment() {
        let td = TypeDescriptor::Numeric {
            code: LvTypeCode::CSgl,
            name: None,
        };
        assert_eq!(td.size(), 8);
        // CSgl alignment = 8/2 = 4
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        assert_eq!(td.alignment(), 4);
    }

    #[test]
    fn complex_dbl_alignment() {
        let td = TypeDescriptor::Numeric {
            code: LvTypeCode::CDbl,
            name: None,
        };
        assert_eq!(td.size(), 16);
        // CDbl alignment = 16/2 = 8
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        assert_eq!(td.alignment(), 8);
    }

    #[test]
    fn offset_of_noncluster_is_none() {
        let td = TypeDescriptor::Numeric {
            code: LvTypeCode::I32,
            name: None,
        };
        assert_eq!(td.offset_of(0), None);
    }

    #[test]
    fn offset_of_out_of_range() {
        let td = TypeDescriptor::Cluster {
            fields: vec![TypeDescriptor::Boolean { name: None }],
            name: None,
        };
        assert_eq!(td.offset_of(5), None);
    }

    #[test]
    fn timestamp_size_and_alignment() {
        let td = TypeDescriptor::Timestamp { name: None };
        assert_eq!(td.size(), 16);
        // Timestamp aligns to 4 (h5labview: "align to U32 not U64")
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        assert_eq!(td.alignment(), 4);
    }

    #[test]
    fn enum_u16_size() {
        let td = TypeDescriptor::Enum {
            base_type: LvTypeCode::EnumU16,
            members: vec!["A".into(), "B".into()],
            name: None,
        };
        assert_eq!(td.size(), 2);
    }

    #[test]
    fn void_size_zero() {
        assert_eq!(TypeDescriptor::Void.size(), 0);
    }

    #[test]
    fn align_to_basics() {
        assert_eq!(align_to(0, 4), 0);
        assert_eq!(align_to(1, 4), 4);
        assert_eq!(align_to(4, 4), 4);
        assert_eq!(align_to(5, 4), 8);
        assert_eq!(align_to(7, 8), 8);
        assert_eq!(align_to(8, 8), 8);
        // alignment 0 or 1 → no change
        assert_eq!(align_to(5, 0), 5);
        assert_eq!(align_to(5, 1), 5);
    }
}
