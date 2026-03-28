//! Test functions for variant access.
//!
//! These functions validate that `LvVariantGetDataPtr` works correctly
//! when called from LabVIEW. Each function receives a Variant handle
//! (by value via "Adapt to Type, Handles by Value"), reads data through
//! the pointer, and returns the value for verification.
//!
//! CLFN setup for the variant parameter:
//!   Type: Adapt to Type
//!   Data format: Handles by Value
//!
//! This means LabVIEW passes the handle directly (not a pointer to it),
//! so the Rust parameter is `LVVariant` by value, not `*const LVVariant`.

use labview_interop::types::{LStrHandle, LVStatusCode, LVVariant};

/// Read a scalar I32 from a Variant.
///
/// LabVIEW test: wire any I32 → `To Variant` → pass Variant to this CLFN.
/// Verify that `result` comes back matching the input.
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_i32(
    variant: LVVariant<'_>,
    result: *mut i32,
) -> LVStatusCode {
    let res = (|| -> labview_interop::errors::Result<()> {
        let data_ptr = variant.data_ptr()?;
        *result = *(data_ptr as *const i32);
        Ok(())
    })();
    res.into()
}

/// Read a scalar DBL (f64) from a Variant.
///
/// LabVIEW test: wire `DBL(3.14)` → `To Variant` → pass Variant to this CLFN.
/// Verify that `result` comes back as `3.14`.
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_dbl(
    variant: LVVariant<'_>,
    result: *mut f64,
) -> LVStatusCode {
    let res = (|| -> labview_interop::errors::Result<()> {
        let data_ptr = variant.data_ptr()?;
        *result = *(data_ptr as *const f64);
        Ok(())
    })();
    res.into()
}

/// Read a scalar Boolean from a Variant.
///
/// LabVIEW test: wire `TRUE` → `To Variant` → pass Variant to this CLFN.
/// Verify that `result` comes back as non-zero.
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_bool(
    variant: LVVariant<'_>,
    result: *mut u8,
) -> LVStatusCode {
    let res = (|| -> labview_interop::errors::Result<()> {
        let data_ptr = variant.data_ptr()?;
        *result = *(data_ptr as *const u8);
        Ok(())
    })();
    res.into()
}

/// Write a scalar I32 into a Variant.
///
/// LabVIEW test:
/// 1. Create I32 Variant via `To Variant` with any I32
/// 2. Pass Variant + value to this CLFN
/// 3. After CLFN returns, use `Variant To Data` wired to I32 → verify value
#[no_mangle]
pub unsafe extern "C" fn test_variant_write_i32(
    variant: LVVariant<'_>,
    value: i32,
) -> LVStatusCode {
    let res = (|| -> labview_interop::errors::Result<()> {
        let data_ptr = variant.data_ptr()?;
        *(data_ptr as *mut i32) = value;
        Ok(())
    })();
    res.into()
}

/// Read a String from a Variant and copy it into the output LStrHandle.
///
/// LabVIEW CLFN setup:
///   variant: Adapt to Type, Handles by Value
///   str_out: String, Handles by Value
///   return:  I32
///
/// The variant's data_ptr for a String points to an LStrHandle.
/// We read through the handle to get the string content and copy it
/// into the output LStrHandle.
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_string(
    variant: LVVariant<'_>,
    mut str_out: LStrHandle<'_>,
) -> LVStatusCode {
    let res = (|| -> labview_interop::errors::Result<()> {
        let data_ptr = variant.data_ptr()?;

        // data_ptr points to an LStrHandle (*mut *mut LStr)
        // We can't move UHandle out of a raw pointer (not Copy),
        // so work with the raw pointer directly.
        let str_handle_ptr = *(data_ptr as *const *mut *mut labview_interop::types::string::LStr);
        if str_handle_ptr.is_null() {
            return Err(labview_interop::errors::InternalError::EmptyVariant.into());
        }
        let inner_ptr = *str_handle_ptr;
        if inner_ptr.is_null() {
            return Err(labview_interop::errors::InternalError::EmptyVariant.into());
        }

        // Read the source string
        let src_lstr = &*inner_ptr;
        let src_bytes = src_lstr.as_slice();

        // Copy into output handle
        str_out.set(src_bytes)?;

        Ok(())
    })();
    res.into()
}
