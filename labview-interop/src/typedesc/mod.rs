//! LabVIEW type descriptor parsing and layout computation.
//!
//! This module parses the binary type descriptor format used by LabVIEW to describe
//! data type memory layouts. It is the Rust equivalent of h5labview's `typeconv.c`.
//!
//! The module is not feature-gated — it is pure parsing logic that works without
//! LabVIEW runtime linkage.
//!
//! # Usage
//!
//! ```
//! use labview_interop::typedesc::{parse, TypeDescriptor, LvTypeCode};
//!
//! // Parse a U32 scalar type descriptor (minimal 4-byte section)
//! let bytes = [0x00, 0x04, 0x00, 0x07]; // len=4, flags=0, code=U32
//! let td = parse(&bytes).unwrap();
//!
//! assert_eq!(td.size(), 4);
//! assert_eq!(td.type_code(), LvTypeCode::U32);
//! ```

mod layout;
mod parser;
mod types;

pub use layout::{align_to, platform_alignment};
pub use parser::{parse, parse_native, parse_with_array, parse_with_order, ByteOrder};
pub use types::{LvTypeCode, PhysicalUnit, TypeDescriptor};
