use std::ffi::c_void;

use crate::memory::UHandle;

#[cfg(feature = "variant")]
use crate::typedesc::{LvTypeCode, TypeDescriptor};

/// Trait for Rust types that can be extracted from a LabVIEW Variant.
///
/// This is the Rust equivalent of LabVIEW's "Variant To Data" — it
/// verifies the variant's type descriptor matches `T` before reading.
///
/// Each implementor defines:
/// - How to check type compatibility ([`is_compatible`](VariantCompatible::is_compatible))
/// - How to extract the value from a raw data pointer ([`read_from_variant_ptr`](VariantCompatible::read_from_variant_ptr))
///
/// # Built-in implementations
///
/// | Rust type | LabVIEW type | Notes |
/// |-----------|-------------|-------|
/// | `i8`..`i64`, `u8`..`u64` | Integer scalars | Bitwise copy via `read_unaligned` |
/// | `f32`, `f64` | SGL, DBL | Bitwise copy via `read_unaligned` |
/// | `LVBool` | Boolean | Bitwise copy via `read_unaligned` |
/// | `String` | String | Dereferences LStr handle, copies to owned `String` |
/// | `Vec<T>` | 1D Array of T | Dereferences array handle, copies elements. `T` must be `VariantCompatible + Copy` |
///
/// # Implementing for clusters
///
/// For LabVIEW clusters containing only scalar fields, define a `repr(C)` struct
/// using [`labview_layout!`](crate::labview_layout) and implement this trait manually.
/// The struct must derive `Copy + Clone` since it is read via `read_unaligned`.
///
/// ```rust,ignore
/// use labview_interop::{labview_layout, types::LVBool};
/// use labview_interop::typedesc::{TypeDescriptor, LvTypeCode};
/// use labview_interop::types::VariantCompatible;
/// use std::ffi::c_void;
///
/// labview_layout!(
///     #[derive(Copy, Clone)]
///     pub struct SensorReading {
///         pub temperature: f64,
///         pub pressure: f64,
///         pub valid: LVBool,
///     }
/// );
///
/// impl VariantCompatible for SensorReading {
///     const TYPE_NAME: &'static str = "Cluster{DBL,DBL,Boolean}";
///
///     fn is_compatible(td: &TypeDescriptor) -> bool {
///         matches!(td, TypeDescriptor::Cluster { fields, .. }
///             if fields.len() == 3
///             && f64::is_compatible(&fields[0])
///             && f64::is_compatible(&fields[1])
///             && LVBool::is_compatible(&fields[2])
///         )
///     }
///
///     unsafe fn read_from_variant_ptr(
///         ptr: *mut c_void,
///     ) -> labview_interop::errors::Result<Self> {
///         Ok(std::ptr::read_unaligned(ptr as *const Self))
///     }
/// }
///
/// // Then in a CLFN:
/// // let reading: SensorReading = unsafe { variant.read_as::<SensorReading>()? };
/// ```
///
/// **Important:** This pattern only works for clusters whose fields are all
/// `Copy` (scalars, booleans, enums). Clusters containing strings, arrays,
/// or other handle-based types require field-by-field extraction — see the
/// `String` and `Vec<T>` implementations for the handle dereferencing pattern.
#[cfg(feature = "variant")]
pub trait VariantCompatible: Sized {
    /// Human-readable type name for error messages.
    const TYPE_NAME: &'static str;

    /// Returns true if the given type descriptor is compatible with this Rust type.
    fn is_compatible(td: &TypeDescriptor) -> bool;

    /// Extract a value from the variant's raw data pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid pointer as returned by `LvVariantGetDataPtr`,
    /// pointing to data whose type matches the result of `is_compatible`.
    unsafe fn read_from_variant_ptr(ptr: *mut c_void) -> crate::errors::Result<Self>;
}

// ---------------------------------------------------------------------------
// Scalar implementations
// ---------------------------------------------------------------------------

#[cfg(feature = "variant")]
macro_rules! impl_variant_scalar {
    ($rust_type:ty, $lv_code:expr, $name:expr) => {
        impl VariantCompatible for $rust_type {
            const TYPE_NAME: &'static str = $name;

            fn is_compatible(td: &TypeDescriptor) -> bool {
                td.type_code() == $lv_code
            }

            unsafe fn read_from_variant_ptr(ptr: *mut c_void) -> crate::errors::Result<Self> {
                Ok(std::ptr::read_unaligned(ptr as *const Self))
            }
        }
    };
}

#[cfg(feature = "variant")]
impl_variant_scalar!(i8,  LvTypeCode::I8,  "I8");
#[cfg(feature = "variant")]
impl_variant_scalar!(i16, LvTypeCode::I16, "I16");
#[cfg(feature = "variant")]
impl_variant_scalar!(i32, LvTypeCode::I32, "I32");
#[cfg(feature = "variant")]
impl_variant_scalar!(i64, LvTypeCode::I64, "I64");
#[cfg(feature = "variant")]
impl_variant_scalar!(u8,  LvTypeCode::U8,  "U8");
#[cfg(feature = "variant")]
impl_variant_scalar!(u16, LvTypeCode::U16, "U16");
#[cfg(feature = "variant")]
impl_variant_scalar!(u32, LvTypeCode::U32, "U32");
#[cfg(feature = "variant")]
impl_variant_scalar!(u64, LvTypeCode::U64, "U64");
#[cfg(feature = "variant")]
impl_variant_scalar!(f32, LvTypeCode::Sgl, "SGL");
#[cfg(feature = "variant")]
impl_variant_scalar!(f64, LvTypeCode::Dbl, "DBL");

#[cfg(feature = "variant")]
impl VariantCompatible for super::LVBool {
    const TYPE_NAME: &'static str = "Boolean";

    fn is_compatible(td: &TypeDescriptor) -> bool {
        td.type_code() == LvTypeCode::Boolean
    }

    unsafe fn read_from_variant_ptr(ptr: *mut c_void) -> crate::errors::Result<Self> {
        Ok(std::ptr::read_unaligned(ptr as *const Self))
    }
}

// ---------------------------------------------------------------------------
// String implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "variant")]
impl VariantCompatible for String {
    const TYPE_NAME: &'static str = "String";

    fn is_compatible(td: &TypeDescriptor) -> bool {
        td.type_code() == LvTypeCode::String
    }

    unsafe fn read_from_variant_ptr(ptr: *mut c_void) -> crate::errors::Result<Self> {
        use crate::errors::InternalError;
        use crate::types::string::LStr;

        // data_ptr for a String variant points to an LStrHandle (*mut *mut LStr).
        // Dereference the handle chain to reach the LStr.
        let handle_ptr = *(ptr as *const *mut *mut LStr);
        if handle_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }
        let lstr_ptr = *handle_ptr;
        if lstr_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }

        let lstr = &*lstr_ptr;
        Ok(lstr.to_rust_string().into_owned())
    }
}

// ---------------------------------------------------------------------------
// Vec<T> implementation for 1D arrays
// ---------------------------------------------------------------------------

#[cfg(feature = "variant")]
impl<T: VariantCompatible + Copy> VariantCompatible for Vec<T> {
    const TYPE_NAME: &'static str = "Array";

    fn is_compatible(td: &TypeDescriptor) -> bool {
        matches!(td, TypeDescriptor::Array { ndims: 1, element, .. }
            if T::is_compatible(element))
    }

    unsafe fn read_from_variant_ptr(ptr: *mut c_void) -> crate::errors::Result<Self> {
        use crate::errors::InternalError;

        // data_ptr for an Array variant points to an LVArrayHandle (*mut *mut LVArray<1, T>).
        // The LVArray layout is: [i32 dim_size] [T; dim_size]
        // We work with raw pointers to avoid alignment issues on Linux/32-bit.

        let handle_ptr = *(ptr as *const *mut *mut u8);
        if handle_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }
        let array_ptr = *handle_ptr;
        if array_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }

        // Read dim_size (i32) from the start of the array struct
        let dim_size = std::ptr::read_unaligned(array_ptr as *const i32);
        if dim_size < 0 {
            return Err(InternalError::BrokenVariant.into());
        }
        let count = dim_size as usize;

        // Data starts after the i32 dim_size field, aligned to the element's
        // LabVIEW alignment (natural alignment capped at the platform max —
        // e.g. f64 data begins at offset 8 on Win64, offset 4 on packed
        // 32-bit). h5labview: `data = DO_ALIGN(data + 4*ndims, align)`.
        let platform = crate::typedesc::platform_alignment().max(1);
        let elem_align = std::mem::align_of::<T>().min(platform);
        let data_offset = crate::typedesc::align_to(std::mem::size_of::<i32>(), elem_align);
        let data_start = (array_ptr as *const u8).add(data_offset);

        let mut result = Vec::with_capacity(count);
        let elem_size = std::mem::size_of::<T>();
        for i in 0..count {
            let elem_ptr = data_start.add(i * elem_size) as *const T;
            result.push(std::ptr::read_unaligned(elem_ptr));
        }

        Ok(result)
    }
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

    /// Reads a value from the variant with type-checking.
    ///
    /// This is the Rust equivalent of LabVIEW's **Variant To Data**.
    /// It calls [`type_descriptor`](LVVariant::type_descriptor) to verify the
    /// variant's type matches `T`, then extracts the value using the
    /// type's [`VariantCompatible::read_from_variant_ptr`] implementation.
    ///
    /// # Supported types
    ///
    /// - **Scalars**: `i8`..`i64`, `u8`..`u64`, `f32`, `f64`, `LVBool`
    /// - **String**: returns an owned `String` (copies from LabVIEW handle)
    /// - **1D arrays**: `Vec<T>` where `T` is a scalar (copies from LabVIEW handle)
    /// - **Clusters**: user-defined `Copy` structs via [`labview_layout!`](crate::labview_layout)
    ///   (see [`VariantCompatible`] trait docs for how to implement)
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
    /// # Examples (inside a CLFN)
    ///
    /// ```rust,ignore
    /// let value: i32 = unsafe { variant.read_as::<i32>()? };
    /// let text: String = unsafe { variant.read_as::<String>()? };
    /// let data: Vec<f64> = unsafe { variant.read_as::<Vec<f64>>()? };
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
        T::read_from_variant_ptr(ptr)
    }

    /// Reads the variant's data into a dynamically-typed [`LvValue`] tree,
    /// guided entirely by the variant's own type descriptor.
    ///
    /// This is the fully dynamic counterpart of [`read_as`](LVVariant::read_as):
    /// no Rust type needs to be declared up front, so a single code path can
    /// handle any supported LabVIEW type — scalars, strings, N-D arrays,
    /// clusters (nested, with mixed field types), timestamps, enums and
    /// refnums. See [`crate::typedesc::read_value`] for the supported set and
    /// [`crate::typedesc::LvValue`] for the canonical string rendering used
    /// by the integration test oracle.
    ///
    /// # Safety
    ///
    /// The variant handle must be valid and alive. The type descriptor and
    /// data pointer are both obtained from the variant itself, so they are
    /// consistent by construction.
    ///
    /// # Errors
    ///
    /// - `VariantApiUnavailable` if the variant API cannot be loaded
    /// - `EmptyVariant` if the handle is null or the variant has no data
    /// - `InvalidTypeDescriptor` / `BrokenVariant` on malformed data
    pub unsafe fn to_value(&self) -> crate::errors::Result<crate::typedesc::LvValue> {
        let td = self.type_descriptor()?;
        // Void variants carry no data — don't ask for a data pointer.
        if matches!(td, crate::typedesc::TypeDescriptor::Void) {
            return Ok(crate::typedesc::LvValue::Void);
        }
        let ptr = self.data_ptr()?;
        crate::typedesc::read_value(&td, ptr)
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
