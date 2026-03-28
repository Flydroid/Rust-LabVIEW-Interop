use std::ffi::c_void;

use crate::memory::UHandle;

#[cfg(feature = "variant")]
use crate::typedesc::{LvTypeCode, TypeDescriptor};

/// Trait for Rust types that can be read from a LabVIEW Variant.
///
/// Implement this for types that have a direct mapping to a LabVIEW
/// type code, enabling type-checked reads via [`LVVariant::read_as`].
///
/// This is the Rust equivalent of LabVIEW's "Variant To Data" — it
/// verifies the variant's type descriptor matches `T` before reading.
#[cfg(feature = "variant")]
pub trait VariantCompatible: Copy {
    /// The expected LabVIEW type code for this Rust type.
    const TYPE_CODE: LvTypeCode;

    /// Human-readable type name for error messages.
    const TYPE_NAME: &'static str;

    /// Returns true if the given type descriptor is compatible with this Rust type.
    ///
    /// The default implementation checks for an exact type code match.
    /// Override this for types that accept multiple type codes (e.g. enums).
    fn is_compatible(td: &TypeDescriptor) -> bool {
        td.type_code() == Self::TYPE_CODE
    }
}

#[cfg(feature = "variant")]
macro_rules! impl_variant_compatible {
    ($rust_type:ty, $lv_code:expr, $name:expr) => {
        impl VariantCompatible for $rust_type {
            const TYPE_CODE: LvTypeCode = $lv_code;
            const TYPE_NAME: &'static str = $name;
        }
    };
}

#[cfg(feature = "variant")]
impl_variant_compatible!(i8,  LvTypeCode::I8,  "I8");
#[cfg(feature = "variant")]
impl_variant_compatible!(i16, LvTypeCode::I16, "I16");
#[cfg(feature = "variant")]
impl_variant_compatible!(i32, LvTypeCode::I32, "I32");
#[cfg(feature = "variant")]
impl_variant_compatible!(i64, LvTypeCode::I64, "I64");
#[cfg(feature = "variant")]
impl_variant_compatible!(u8,  LvTypeCode::U8,  "U8");
#[cfg(feature = "variant")]
impl_variant_compatible!(u16, LvTypeCode::U16, "U16");
#[cfg(feature = "variant")]
impl_variant_compatible!(u32, LvTypeCode::U32, "U32");
#[cfg(feature = "variant")]
impl_variant_compatible!(u64, LvTypeCode::U64, "U64");
#[cfg(feature = "variant")]
impl_variant_compatible!(f32, LvTypeCode::Sgl, "SGL");
#[cfg(feature = "variant")]
impl_variant_compatible!(f64, LvTypeCode::Dbl, "DBL");
#[cfg(feature = "variant")]
impl VariantCompatible for super::LVBool {
    const TYPE_CODE: LvTypeCode = LvTypeCode::Boolean;
    const TYPE_NAME: &'static str = "Boolean";
}

/// Represents a LabVIEW Variant. The internal structure is undefined
/// by NI and therefore unavailable.
///
/// This is available as a placeholder in clusters etc.
///
/// With the `variant` feature enabled, the [`LVVariant::data_ptr`] method provides
/// zero-copy access to the variant's underlying data via the undocumented
/// `LvVariantGetDataPtr` runtime function. The type descriptor can be
/// obtained via [`LVVariant::type_descriptor`] and parsed via the `typedesc`
/// module to understand the data layout.
#[repr(transparent)]
pub struct LVVariant<'variant>(UHandle<'variant, c_void>);

impl LVVariant<'_> {
    /// Returns true if the variant handle is null.
    pub fn is_null(&self) -> bool {
        self.0 .0.is_null()
    }
}

#[cfg(feature = "variant")]
impl LVVariant<'_> {
    /// Dereferences the UHandle once and returns the inner pointer,
    /// or an error if the handle is null.
    unsafe fn inner_ptr(&self) -> crate::errors::Result<*mut c_void> {
        use crate::errors::InternalError;
        if self.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }
        let inner = *self.0 .0;
        if inner.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }
        Ok(inner)
    }

    /// Returns true if the variant is empty (contains no data).
    ///
    /// Uses the undocumented `LvVariantIsEmpty` LabVIEW runtime function.
    ///
    /// # Safety
    ///
    /// The variant handle must be valid and alive.
    ///
    /// # Errors
    ///
    /// - `VariantApiUnavailable` if the variant API cannot be loaded
    /// - `EmptyVariant` if the handle itself is null
    pub unsafe fn is_empty(&self) -> crate::errors::Result<bool> {
        use crate::errors::InternalError;
        let api = crate::labview::variant_api()
            .map_err(|_| InternalError::VariantApiUnavailable)?;
        let inner = self.inner_ptr()?;
        Ok(api.variant_is_empty(inner) != 0)
    }

    /// Returns the parsed type descriptor for this variant.
    ///
    /// Calls `GetTypeFromLvVariant` to obtain the in-memory type descriptor
    /// bytes (native endian format), reads the section length, copies the bytes,
    /// and parses them via [`crate::typedesc::parse_native`].
    ///
    /// # Safety
    ///
    /// The variant handle must be valid and alive.
    ///
    /// # Errors
    ///
    /// - `VariantApiUnavailable` if the variant API cannot be loaded
    /// - `EmptyVariant` if the handle is null or the variant is empty
    /// - `InvalidTypeDescriptor` if the type descriptor bytes cannot be parsed
    pub unsafe fn type_descriptor(&self) -> crate::errors::Result<crate::typedesc::TypeDescriptor> {
        use crate::errors::InternalError;
        let api = crate::labview::variant_api()
            .map_err(|_| InternalError::VariantApiUnavailable)?;
        let inner = self.inner_ptr()?;

        let td_ptr = api.get_type_from_variant(inner);
        if td_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }

        // First two bytes are the section length in native endianness
        let len = u16::from_ne_bytes([*td_ptr, *td_ptr.add(1)]) as usize;
        if len < 4 {
            return Err(InternalError::BrokenVariant.into());
        }

        // Copy the type descriptor bytes to an owned buffer
        let td_bytes = std::slice::from_raw_parts(td_ptr, len);
        crate::typedesc::parse_native(td_bytes).map_err(Into::into)
    }

    /// Returns a raw mutable pointer to the variant's underlying data.
    ///
    /// Uses the undocumented `LvVariantGetDataPtr` LabVIEW runtime function.
    /// The caller must use a parsed `TypeDescriptor` to interpret the memory layout.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid while the variant handle is alive.
    /// The caller must ensure correct interpretation based on the type descriptor.
    ///
    /// # Errors
    ///
    /// - `VariantApiUnavailable` if `LvVariantGetDataPtr` cannot be loaded
    /// - `EmptyVariant` if the handle is null or the data pointer is null
    pub unsafe fn data_ptr(&self) -> crate::errors::Result<*mut c_void> {
        use crate::errors::InternalError;

        let api = crate::labview::variant_api()
            .map_err(|_| InternalError::VariantApiUnavailable)?;

        let inner = self.inner_ptr()?;

        let result = api.variant_get_data_ptr(inner);

        if result.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }

        Ok(result)
    }

    /// Reads a scalar value from the variant with type-checking.
    ///
    /// This is the Rust equivalent of LabVIEW's **Variant To Data**.
    /// It calls [`type_descriptor`](LVVariant::type_descriptor) to verify the
    /// variant's type matches `T`, then reads the value using `read_unaligned`
    /// (safe on all platforms regardless of LabVIEW alignment rules).
    ///
    /// # Safety
    ///
    /// The variant handle must be valid and alive.
    ///
    /// # Errors
    ///
    /// - `VariantTypeMismatch` if the variant's type doesn't match `T`
    /// - `VariantApiUnavailable` if the variant API cannot be loaded
    /// - `EmptyVariant` if the variant is null or empty
    ///
    /// # Example (inside a CLFN)
    ///
    /// ```rust,ignore
    /// let value: i32 = unsafe { variant.read_as::<i32>()? };
    /// ```
    pub unsafe fn read_as<T: VariantCompatible>(&self) -> crate::errors::Result<T> {
        use crate::errors::InternalError;

        let td = self.type_descriptor()?;
        if !T::is_compatible(&td) {
            return Err(InternalError::VariantTypeMismatch {
                expected: T::TYPE_NAME,
                found: td.type_code().as_str(),
            }
            .into());
        }

        let ptr = self.data_ptr()?;
        Ok(std::ptr::read_unaligned(ptr as *const T))
    }

    /// Returns a typed reference to the variant's data, interpreting the raw
    /// data pointer as type `T`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `T` matches the actual variant data layout exactly
    /// - The variant handle is valid and alive
    /// - The data is properly aligned for `T`
    ///
    /// # Note
    ///
    /// Prefer [`read_as`](LVVariant::read_as) for scalar types — it validates
    /// the type and handles alignment safely. This method is provided for
    /// advanced use cases where you need a reference to complex data.
    pub unsafe fn as_typed_ref<T>(&self) -> crate::errors::Result<&T> {
        let ptr = self.data_ptr()?;
        let typed_ptr = ptr as *const T;
        typed_ptr
            .as_ref()
            .ok_or_else(|| crate::errors::InternalError::EmptyVariant.into())
    }
}
