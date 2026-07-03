//! The sync module provides access to the
//! functions which allow for synchronising
//! back to labview.
//!

use std::ffi::c_void;
use std::marker::PhantomData;

use crate::errors::Result;
use crate::labview::sync_api;
use crate::memory::MagicCookie;

type LVUserEventRef = MagicCookie;

/// Representation of a LabVIEW user event reference with type data.
///
/// Where the reference is passed into Rust you can use this typed form
/// to then allow proper type completions of the values.
///
/// From LabVIEW you can set the terminal to be `adapt to type` and `handles by value`
///
/// # Example
/// ```
/// # use labview_interop::sync::LVUserEvent;
/// # use labview_interop::types::LVStatusCode;
///#[no_mangle]
///pub extern "C" fn generate_event_3(lv_user_event: *mut LVUserEvent<i32>) -> LVStatusCode {
///    let event = unsafe { *lv_user_event };
///    let result = event.post(&mut 3);
///    match result {
///        Ok(_) => LVStatusCode::SUCCESS,
///        Err(err) => err.into(),
///    }
///}
/// ```
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct LVUserEvent<T> {
    reference: LVUserEventRef,
    _marker: PhantomData<T>,
}

impl<T> LVUserEvent<T> {
    /// Generate the user event with the provided data.
    ///
    /// Right now the data needs to be a mutable reference as the
    /// LabVIEW API does not specify whether it will not be modified.
    ///
    /// # Ownership warning
    ///
    /// `PostLVUserEvent` makes a **shallow copy** of the data: it copies the raw
    /// bytes of `T` (including any handle pointer values it contains) into the
    /// event queue.  For plain `Copy` types (integers, floats, booleans) this is
    /// fine.  For types that contain LabVIEW-allocated handles (e.g. `LStrOwned`
    /// inside a cluster) the caller **must** call [`std::mem::forget`] on `data`
    /// after a successful post so that Rust does not free those handles while
    /// LabVIEW's event queue still holds the pointers.
    ///
    /// Prefer [`LVUserEvent::post_owned`] for any `T` that owns LabVIEW handles.
    pub fn post(&self, data: &mut T) -> Result<()> {
        let mg_err = unsafe {
            sync_api()?.post_lv_user_event(self.reference, data as *mut T as *mut c_void)
        };
        mg_err.to_specific_result(())
    }

    /// Generate the user event, transferring ownership of `data` to LabVIEW.
    ///
    /// This is the correct method to use when `T` contains LabVIEW-allocated
    /// handles such as `LStrOwned`. `PostLVUserEvent` makes a shallow copy of
    /// the cluster bytes (including the handle pointer values). On success this
    /// function calls [`std::mem::forget`] so that Rust does not free those
    /// handles — LabVIEW owns them and will free them when the event is consumed
    /// by the Event Structure.
    ///
    /// On failure the post did not succeed, so `data` is dropped normally and
    /// the handles are freed by Rust.
    ///
    /// # Example
    /// ```no_run
    /// # use labview_interop::sync::LVUserEvent;
    /// # use labview_interop::types::LVStatusCode;
    /// # use labview_interop::labview_layout;
    /// # use labview_interop::types::LStrOwned;
    /// labview_layout!(
    ///     pub struct MyEvent {
    ///         pub message: LStrOwned,
    ///     }
    /// );
    /// #[no_mangle]
    /// pub extern "C" fn send_event(lv_user_event: *mut LVUserEvent<MyEvent>) -> LVStatusCode {
    ///     let event = unsafe { &*lv_user_event };
    ///     let data = MyEvent {
    ///         message: LStrOwned::from_data(b"hello").unwrap(),
    ///     };
    ///     match event.post_owned(data) {
    ///         Ok(_) => LVStatusCode::SUCCESS,
    ///         Err(err) => err.into(),
    ///     }
    /// }
    /// ```
    pub fn post_owned(&self, data: T) -> Result<()> {
        let mg_err = unsafe {
            sync_api()?.post_lv_user_event(self.reference, &data as *const T as *mut c_void)
        };
        let result = mg_err.to_specific_result(());
        if result.is_ok() {
            // Transfer ownership of any LabVIEW-allocated handles to LabVIEW.
            // LabVIEW frees them when the event is consumed by the Event Structure.
            std::mem::forget(data);
        }
        // On failure `data` drops here, correctly freeing any handles.
        result
    }
}

/// A LabVIEW occurrence which can be used to provide synchronisation
/// between execution of Rust and LabVIEW code.
///
/// From LabVIEW you can set the terminal to be `adapt to type` and `handles by value`
///
/// # Example
/// ```
/// # use labview_interop::sync::Occurence;
/// # use labview_interop::types::LVStatusCode;
/// #[no_mangle]
///pub extern "C" fn generate_occurence(occurence: *mut Occurence) -> LVStatusCode {
///    let result = unsafe { (*occurence).set() };
///    match result {
///        Ok(_) => LVStatusCode::SUCCESS,
///        Err(err) => err.into(),
///    }
///}
/// ```
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Occurence(MagicCookie);

impl Occurence {
    /// "set" generates the occurrence event which can be detected by LabVIEW.
    pub fn set(&self) -> Result<()> {
        let mg_err = unsafe { sync_api()?.occur(self.0) };
        mg_err.to_specific_result(())
    }
}
