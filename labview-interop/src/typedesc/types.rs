//! Type code enumeration and type descriptor tree for LabVIEW type descriptors.
//!
//! The type descriptor is a binary format that describes the memory layout of
//! LabVIEW data types. It is used by LabVIEW internally and can be extracted
//! from Variants via "Variant To Flattened String" → type cast.
//!
//! Reference: h5labview `typeconv.c:222` (`recurse_typedesc`)

use num_enum::TryFromPrimitive;

/// LabVIEW type codes as found in type descriptor binary sections.
///
/// Reference: h5labview `typeconv.c` switch statements and
/// labview-variant-data `type_converters.py`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[repr(u8)]
pub enum LvTypeCode {
    Void = 0x00,
    I8 = 0x01,
    I16 = 0x02,
    I32 = 0x03,
    I64 = 0x04,
    U8 = 0x05,
    U16 = 0x06,
    U32 = 0x07,
    U64 = 0x08,
    Sgl = 0x09,
    Dbl = 0x0A,
    Ext = 0x0B,
    CSgl = 0x0C,
    CDbl = 0x0D,
    CExt = 0x0E,
    /// Enum with U8 base
    EnumU8 = 0x15,
    /// Enum with U16 base
    EnumU16 = 0x16,
    /// Enum with U32 base
    EnumU32 = 0x17,
    /// Physical quantity with SGL base (code - 0x10 = 0x09)
    PhysSgl = 0x19,
    /// Physical quantity with DBL base (code - 0x10 = 0x0A)
    PhysDbl = 0x1A,
    /// Physical quantity with EXT base (code - 0x10 = 0x0B)
    PhysExt = 0x1B,
    /// Physical quantity with CSG base (code - 0x10 = 0x0C)
    PhysCSgl = 0x1C,
    /// Physical quantity with CDB base (code - 0x10 = 0x0D)
    PhysCDbl = 0x1D,
    /// Physical quantity with CEX base (code - 0x10 = 0x0E)
    PhysCExt = 0x1E,
    Boolean = 0x21,
    String = 0x30,
    Path = 0x32,
    Array = 0x40,
    Cluster = 0x50,
    Variant = 0x53,
    Waveform = 0x54,
    /// Refnum (DVR, queue, event, …). Data-bearing refnums (e.g. DVRs)
    /// carry a nested type descriptor for the referenced type.
    Refnum = 0x70,
    Set = 0x73,
    Map = 0x74,
}

impl LvTypeCode {
    /// Returns true if the type code represents a scalar numeric type (I8..CExt).
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
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
                | LvTypeCode::CExt
        )
    }

    /// Returns true if the type code represents an enum type.
    pub fn is_enum(self) -> bool {
        matches!(
            self,
            LvTypeCode::EnumU8 | LvTypeCode::EnumU16 | LvTypeCode::EnumU32
        )
    }

    /// Returns true if the type code represents a physical quantity.
    pub fn is_physical_quantity(self) -> bool {
        matches!(
            self,
            LvTypeCode::PhysSgl
                | LvTypeCode::PhysDbl
                | LvTypeCode::PhysExt
                | LvTypeCode::PhysCSgl
                | LvTypeCode::PhysCDbl
                | LvTypeCode::PhysCExt
        )
    }

    /// For physical quantity codes, returns the underlying numeric type code.
    /// E.g., `PhysDbl` (0x1A) → `Dbl` (0x0A).
    pub fn physical_to_numeric(self) -> Option<LvTypeCode> {
        let raw = self as u8;
        if (0x19..=0x1E).contains(&raw) {
            LvTypeCode::try_from(raw - 0x10).ok()
        } else {
            None
        }
    }

    /// Returns a human-readable name for this type code.
    pub const fn as_str(self) -> &'static str {
        match self {
            LvTypeCode::Void => "Void",
            LvTypeCode::I8 => "I8",
            LvTypeCode::I16 => "I16",
            LvTypeCode::I32 => "I32",
            LvTypeCode::I64 => "I64",
            LvTypeCode::U8 => "U8",
            LvTypeCode::U16 => "U16",
            LvTypeCode::U32 => "U32",
            LvTypeCode::U64 => "U64",
            LvTypeCode::Sgl => "SGL",
            LvTypeCode::Dbl => "DBL",
            LvTypeCode::Ext => "EXT",
            LvTypeCode::CSgl => "CSG",
            LvTypeCode::CDbl => "CDB",
            LvTypeCode::CExt => "CEX",
            LvTypeCode::EnumU8 => "EnumU8",
            LvTypeCode::EnumU16 => "EnumU16",
            LvTypeCode::EnumU32 => "EnumU32",
            LvTypeCode::PhysSgl => "PhysSGL",
            LvTypeCode::PhysDbl => "PhysDBL",
            LvTypeCode::PhysExt => "PhysEXT",
            LvTypeCode::PhysCSgl => "PhysCSG",
            LvTypeCode::PhysCDbl => "PhysCDB",
            LvTypeCode::PhysCExt => "PhysCEX",
            LvTypeCode::Boolean => "Boolean",
            LvTypeCode::String => "String",
            LvTypeCode::Path => "Path",
            LvTypeCode::Array => "Array",
            LvTypeCode::Cluster => "Cluster",
            LvTypeCode::Variant => "Variant",
            LvTypeCode::Waveform => "Waveform",
            LvTypeCode::Refnum => "Refnum",
            LvTypeCode::Set => "Set",
            LvTypeCode::Map => "Map",
        }
    }
}

/// A unit component in a physical quantity type descriptor.
/// Each unit has a code (radians=0, steradians=1, seconds=2, meters=3,
/// kilograms=4, amperes=5, kelvin=6, moles=7, candela=8) and a power exponent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalUnit {
    /// SI unit code (0=rad, 1=sr, 2=s, 3=m, 4=kg, 5=A, 6=K, 7=mol, 8=cd)
    pub unit: u16,
    /// Power exponent (e.g. 1 for m, -2 for s^-2)
    pub power: i16,
}

/// A parsed LabVIEW type descriptor tree.
///
/// Each variant represents a LabVIEW data type as parsed from the binary
/// type descriptor format. The tree structure mirrors the recursive nature
/// of the format (clusters contain fields, arrays contain element types, etc.).
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDescriptor {
    /// Scalar numeric type (I8, I16, I32, I64, U8..U64, SGL, DBL, EXT, CSG, CDB, CEX)
    Numeric {
        code: LvTypeCode,
        name: Option<String>,
    },
    /// Boolean (1 byte, LabVIEW code 0x21)
    Boolean { name: Option<String> },
    /// LabVIEW string (handle type, code 0x30)
    String { name: Option<String> },
    /// Enumeration with member names
    Enum {
        /// Base storage type (EnumU8, EnumU16, or EnumU32)
        base_type: LvTypeCode,
        /// Enum member names in order
        members: Vec<String>,
        name: Option<String>,
    },
    /// N-dimensional array of a single element type
    Array {
        /// Number of dimensions
        ndims: u16,
        /// Element type
        element: Box<TypeDescriptor>,
        name: Option<String>,
    },
    /// Cluster (record/struct) with ordered fields
    Cluster {
        fields: Vec<TypeDescriptor>,
        name: Option<String>,
    },
    /// Waveform (analog signal)
    Waveform {
        /// Waveform subcode — 6 = timestamp, others map to numeric type
        subcode: u8,
        name: Option<String>,
    },
    /// Timestamp (LabVIEW epoch 1904-01-01, i64 seconds + u64 fractional)
    Timestamp { name: Option<String> },
    /// Physical quantity (numeric with SI units)
    PhysicalQuantity {
        /// Underlying numeric type code
        base_type: LvTypeCode,
        /// SI unit components
        units: Vec<PhysicalUnit>,
        name: Option<String>,
    },
    /// LabVIEW path type (code 0x32)
    Path { name: Option<String> },
    /// LabVIEW Variant type (code 0x53, opaque)
    Variant { name: Option<String> },
    /// Refnum (code 0x70) — DVR, queue, event, etc.
    ///
    /// The runtime data for a refnum is a 4-byte magic cookie.
    /// Data-bearing refnum kinds (e.g. a DVR) carry a nested type descriptor
    /// for the referenced type, observed empirically for external DVRs
    /// (see docs/variant-implementation-plan.md, "Verified Findings").
    Refnum {
        /// Refnum kind word as found in the descriptor (e.g. 0x0020 for an
        /// external DVR). Semantics are not fully mapped yet.
        kind: u16,
        /// The type the reference points at (a DVR's element type).
        /// `None` for refnum kinds that carry no data type.
        referenced: Option<Box<TypeDescriptor>>,
        name: Option<String>,
    },
    /// Map (dictionary) with key and value types (code 0x74)
    Map {
        key: Box<TypeDescriptor>,
        value: Box<TypeDescriptor>,
        name: Option<String>,
    },
    /// Set of elements (code 0x73)
    Set {
        element: Box<TypeDescriptor>,
        name: Option<String>,
    },
    /// Void / Nil — no data (code 0x00)
    Void,
}

impl TypeDescriptor {
    /// Returns the optional name of this type descriptor element.
    pub fn name(&self) -> Option<&str> {
        match self {
            TypeDescriptor::Numeric { name, .. }
            | TypeDescriptor::Boolean { name, .. }
            | TypeDescriptor::String { name, .. }
            | TypeDescriptor::Enum { name, .. }
            | TypeDescriptor::Array { name, .. }
            | TypeDescriptor::Cluster { name, .. }
            | TypeDescriptor::Waveform { name, .. }
            | TypeDescriptor::Timestamp { name, .. }
            | TypeDescriptor::PhysicalQuantity { name, .. }
            | TypeDescriptor::Path { name, .. }
            | TypeDescriptor::Variant { name, .. }
            | TypeDescriptor::Refnum { name, .. }
            | TypeDescriptor::Map { name, .. }
            | TypeDescriptor::Set { name, .. } => name.as_deref(),
            TypeDescriptor::Void => None,
        }
    }

    /// Returns the type code for this descriptor.
    pub fn type_code(&self) -> LvTypeCode {
        match self {
            TypeDescriptor::Numeric { code, .. } => *code,
            TypeDescriptor::Boolean { .. } => LvTypeCode::Boolean,
            TypeDescriptor::String { .. } => LvTypeCode::String,
            TypeDescriptor::Enum { base_type, .. } => *base_type,
            TypeDescriptor::Array { .. } => LvTypeCode::Array,
            TypeDescriptor::Cluster { .. } => LvTypeCode::Cluster,
            TypeDescriptor::Waveform { .. } => LvTypeCode::Waveform,
            TypeDescriptor::Timestamp { .. } => LvTypeCode::Waveform, // timestamp is subcode 6 of waveform
            TypeDescriptor::PhysicalQuantity { base_type, .. } => *base_type,
            TypeDescriptor::Path { .. } => LvTypeCode::Path,
            TypeDescriptor::Variant { .. } => LvTypeCode::Variant,
            TypeDescriptor::Refnum { .. } => LvTypeCode::Refnum,
            TypeDescriptor::Map { .. } => LvTypeCode::Map,
            TypeDescriptor::Set { .. } => LvTypeCode::Set,
            TypeDescriptor::Void => LvTypeCode::Void,
        }
    }
}
