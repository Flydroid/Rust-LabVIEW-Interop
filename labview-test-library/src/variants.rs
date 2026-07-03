//! Test functions for variant access.
//!
//! The primary entry points are the *generic* test-oracle exports, which
//! handle ANY LabVIEW type through one code path so the LabVIEW test VIs can
//! be data-driven (arrays of {variant, expected string} cases) instead of
//! one VI per type:
//!
//! - [`variant_to_canonical_string`] — renders the variant's contents as the
//!   canonical string documented in `labview_interop::typedesc::value`.
//! - [`variant_typedesc_hex`] — hex dump of the raw `GetTypeFromLvVariant`
//!   bytes (lowercase, no separators), for golden-vector capture.
//! - [`variant_dbl_array_sum`] — zero-copy sum/length of a 1-D DBL array,
//!   for the large-dataset test where rendering 1M elements as a string
//!   would be impractical.
//!
//! The `test_variant_read_*` scalar probes remain for the original
//! single-type smoke tests.
//!
//! CLFN setup for the variant parameter (all functions):
//!   Type: Adapt to Type
//!   Data format: Handles by Value
//!
//! This means LabVIEW passes the handle directly (not a pointer to it),
//! so the Rust parameter is `LVVariant` by value, not `*const LVVariant`.
//!
//! String outputs are `String` parameters configured as Handles by Value.
//!
//! All exports catch panics at the FFI boundary — a panic returns status
//! 542099 instead of unwinding into LabVIEW (which is undefined behaviour).

use labview_interop::errors::InternalError;
use labview_interop::types::{LStrHandle, LVStatusCode, LVVariant};
use std::panic::AssertUnwindSafe;

/// Status code returned when a test function panics. In the crate's custom
/// error range (542,000+) but outside the codes used by labview-interop.
const PANIC_STATUS: i32 = 542_099;

/// Run `body`, converting a returned error into its status code and a panic
/// into [`PANIC_STATUS`], so nothing unwinds across the FFI boundary.
fn catch_status(body: impl FnOnce() -> labview_interop::errors::Result<()>) -> LVStatusCode {
    match std::panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result.into(),
        Err(_) => LVStatusCode::from(PANIC_STATUS),
    }
}

/// Render any variant's contents as the canonical string (the integration
/// test oracle). See `labview_interop::typedesc::value` for the format.
///
/// CLFN: variant = Adapt to Type / Handles by Value;
///       result  = String / Handles by Value; return = I32.
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn variant_to_canonical_string(
    variant: LVVariant<'_>,
    mut result: LStrHandle<'_>,
) -> LVStatusCode {
    catch_status(|| {
        let value = variant.to_value()?;
        result.set(value.to_string().as_bytes())?;
        Ok(())
    })
}

/// Write the variant's raw in-memory type descriptor bytes as a lowercase
/// hex string (no separators) — for golden-vector capture.
///
/// CLFN: variant = Adapt to Type / Handles by Value;
///       result  = String / Handles by Value; return = I32.
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn variant_typedesc_hex(
    variant: LVVariant<'_>,
    mut result: LStrHandle<'_>,
) -> LVStatusCode {
    catch_status(|| {
        let bytes = variant.type_descriptor_bytes()?;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        result.set(hex.as_bytes())?;
        Ok(())
    })
}

/// Zero-copy check for large arrays: sums a 1-D DBL array variant in place
/// and reports its length. No per-element materialization — the element
/// data is read as one `&[f64]` slice straight out of the variant's memory,
/// so VI-side timing of this call measures the zero-copy path.
///
/// CLFN: variant = Adapt to Type / Handles by Value;
///       sum = Numeric f64 pointer; len = Numeric u64 pointer; return = I32.
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn variant_dbl_array_sum(
    variant: LVVariant<'_>,
    sum: *mut f64,
    len: *mut u64,
) -> LVStatusCode {
    use labview_interop::typedesc::{align_to, platform_alignment, LvTypeCode, TypeDescriptor};

    catch_status(|| {
        let td = variant.type_descriptor()?;
        let is_1d_dbl = matches!(
            &td,
            TypeDescriptor::Array { ndims: 1, element, .. }
                if element.type_code() == LvTypeCode::Dbl
        );
        if !is_1d_dbl {
            return Err(InternalError::VariantTypeMismatch {
                expected: "1-D DBL Array",
                found: td.type_code().as_str(),
            }
            .into());
        }

        let ptr = variant.data_ptr()?;
        let handle = *(ptr as *const *mut *mut u8);
        if handle.is_null() || (*handle).is_null() {
            *sum = 0.0;
            *len = 0;
            return Ok(());
        }
        let data = *handle;

        let n = std::ptr::read_unaligned(data as *const i32);
        if n < 0 {
            return Err(InternalError::BrokenVariant.into());
        }
        // Element data starts after the dimension word, aligned for f64.
        let elem_align = 8usize.min(platform_alignment().max(1));
        let start = align_to(4, elem_align);
        let elements = std::slice::from_raw_parts(data.add(start) as *const f64, n as usize);

        *sum = elements.iter().sum();
        *len = n as u64;
        Ok(())
    })
}

/// Read a scalar I32 from a Variant.
///
/// LabVIEW test: wire any I32 → `To Variant` → pass Variant to this CLFN.
/// Verify that `result` comes back matching the input.
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_i32(
    variant: LVVariant<'_>,
    result: *mut i32,
) -> LVStatusCode {
    catch_status(|| {
        let data_ptr = variant.data_ptr()?;
        *result = *(data_ptr as *const i32);
        Ok(())
    })
}

/// Read a scalar DBL (f64) from a Variant.
///
/// LabVIEW test: wire `DBL(3.14)` → `To Variant` → pass Variant to this CLFN.
/// Verify that `result` comes back as `3.14`.
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_dbl(
    variant: LVVariant<'_>,
    result: *mut f64,
) -> LVStatusCode {
    catch_status(|| {
        let data_ptr = variant.data_ptr()?;
        *result = *(data_ptr as *const f64);
        Ok(())
    })
}

/// Read a scalar Boolean from a Variant.
///
/// LabVIEW test: wire `TRUE` → `To Variant` → pass Variant to this CLFN.
/// Verify that `result` comes back as non-zero.
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_bool(
    variant: LVVariant<'_>,
    result: *mut u8,
) -> LVStatusCode {
    catch_status(|| {
        let data_ptr = variant.data_ptr()?;
        *result = *(data_ptr as *const u8);
        Ok(())
    })
}

/// Write a scalar I32 into a Variant.
///
/// LabVIEW test:
/// 1. Create I32 Variant via `To Variant` with any I32
/// 2. Pass Variant + value to this CLFN
/// 3. After CLFN returns, use `Variant To Data` wired to I32 → verify value
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn test_variant_write_i32(
    variant: LVVariant<'_>,
    value: i32,
) -> LVStatusCode {
    catch_status(|| {
        let data_ptr = variant.data_ptr()?;
        *(data_ptr as *mut i32) = value;
        Ok(())
    })
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
///
/// # Safety
///
/// Must only be called by LabVIEW through a CLFN configured as documented
/// above, with a valid, live variant handle (and valid output pointers).
#[no_mangle]
pub unsafe extern "C" fn test_variant_read_string(
    variant: LVVariant<'_>,
    mut str_out: LStrHandle<'_>,
) -> LVStatusCode {
    catch_status(|| {
        let data_ptr = variant.data_ptr()?;

        // data_ptr points to an LStrHandle (*mut *mut LStr)
        // We can't move UHandle out of a raw pointer (not Copy),
        // so work with the raw pointer directly.
        let str_handle_ptr = *(data_ptr as *const *mut *mut labview_interop::types::string::LStr);
        if str_handle_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }
        let inner_ptr = *str_handle_ptr;
        if inner_ptr.is_null() {
            return Err(InternalError::EmptyVariant.into());
        }

        // Read the source string
        let src_lstr = &*inner_ptr;
        let src_bytes = src_lstr.as_slice();

        // Copy into output handle
        str_out.set(src_bytes)?;

        Ok(())
    })
}
