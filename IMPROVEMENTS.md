# Proposed Improvements for Better LabVIEW Integration

This document outlines proposed improvements to the `labview-interop` crate for deeper, more robust integration with LabVIEW features. These are organized by priority and grouped by feature area.

---

## Table of Contents

1. [Cluster Support Improvements](#1-cluster-support-improvements)
2. [Missing LabVIEW Type Support](#2-missing-labview-type-support)
3. [Memory Management Enhancements](#3-memory-management-enhancements)
4. [Synchronization & Communication](#4-synchronization--communication)
5. [Error Handling Improvements](#5-error-handling-improvements)
6. [32-Bit Platform Improvements](#6-32-bit-platform-improvements)
7. [Developer Experience & API Ergonomics](#7-developer-experience--api-ergonomics)
8. [Testing & Safety](#8-testing--safety)
9. [Documentation & Examples](#9-documentation--examples)

---

## 1. Cluster Support Improvements

### Current State

Clusters are supported via the `labview_layout!` macro which applies `#[repr(C)]` on 64-bit and `#[repr(C, packed)]` on 32-bit. Users manually define Rust structs that must match LabVIEW cluster wire layout.

**What works:**
- Simple clusters with numeric types, strings, arrays, timestamps
- Nested clusters containing `LStrHandle`, `LVArrayHandle`, `Waveform`, `LVVariant`
- `ErrorCluster` as a built-in cluster type (64-bit only)
- Clusters as user event payloads

**What doesn't work or is limited:**
- Error clusters are 64-bit only (due to unaligned pointer access restrictions)
- No automatic validation that Rust struct layout matches LabVIEW cluster layout
- No deep clone for clusters containing handles (only shallow copy via `DSCopyHandle`)
- No derive macro for automatic `labview_layout` application
- Waveform padding is reverse-engineered with no official documentation

### Proposed Improvements

#### 1.1 — Extend Error Clusters to 32-Bit ⭐⭐⭐
**Problem:** `ErrorCluster` and `ErrorClusterPtr` are gated behind `#[cfg(target_pointer_width = "64")]`, making error handling impossible on 32-bit targets.

**Proposal:** Implement 32-bit error cluster support using `read_unaligned`/`write_unaligned` access patterns already demonstrated for other 32-bit cluster fields. The `ErrorCluster` struct uses `labview_layout!` already, so the data layout is correct — only the accessor methods need safe unaligned wrappers.

#### 1.2 — Derive Macro for Cluster Structs ⭐⭐
**Problem:** Users must manually wrap every cluster struct in `labview_layout!()` and cannot get compile-time validation that their struct matches the LabVIEW wire format.

**Proposal:** Create a `#[derive(LabVIEWCluster)]` proc macro that:
- Automatically applies `repr(C)` / `repr(C, packed)` based on target
- Validates field types are LabVIEW-compatible at compile time
- Generates safe accessor methods for 32-bit (unaligned read/write wrappers)
- Optionally generates `size_of` validation against expected LabVIEW cluster size

#### 1.3 — Deep Clone for Clusters with Handles ⭐⭐
**Problem:** `DSCopyHandle` performs shallow copy. Clusters containing `LStrHandle` or `LVArrayHandle` fields get their handles duplicated but the inner data shared — leading to double-free or use-after-free risks.

**Proposal:** Implement a `DeepClone` trait (or extend `LVCopy`) that recursively clones all handle fields within a cluster. This could be auto-derived alongside `LabVIEWCluster`.

#### 1.4 — Cluster Size/Layout Validation Utility ⭐
**Problem:** There's no way to verify at runtime that a Rust struct matches the LabVIEW cluster it's supposed to represent.

**Proposal:** Add a `validate_cluster_layout::<T>(expected_size: usize)` function that compares `std::mem::size_of::<T>()` against the expected LabVIEW cluster size (passed from LabVIEW at init time or hardcoded). This catches layout mismatches early.

---

## 2. Missing LabVIEW Type Support

### 2.1 — Enum Types ⭐⭐⭐
**Problem:** LabVIEW enums (ring controls) are commonly used but have no typed representation.

**Proposal:** Add an `LVEnum<T: Into<u16>>` wrapper or a derive macro `#[derive(LabVIEWEnum)]` that:
- Maps Rust enums to LabVIEW U16 ring/enum values
- Validates conversion in both directions
- Supports custom discriminant values

```rust
#[derive(LabVIEWEnum)]
#[repr(u16)]
pub enum MeasurementMode {
    Voltage = 0,
    Current = 1,
    Resistance = 2,
}
```

### 2.2 — Complex Numbers ⭐⭐
**Problem:** LabVIEW has native complex single (CSG) and complex double (CDB) types used heavily in signal processing. These aren't represented.

**Proposal:** Add `LVComplexSingle` and `LVComplexDouble` types:
```rust
#[repr(C)]
pub struct LVComplexDouble {
    pub real: f64,
    pub imag: f64,
}
```
With optional `num-complex` feature gate for interop with the `num` crate ecosystem.

### 2.3 — Path Type ⭐⭐
**Problem:** LabVIEW paths are distinct from strings and have platform-specific encoding.

**Proposal:** Add `LVPath` type wrapping the LabVIEW path handle, with conversion to/from `std::path::PathBuf` and `&Path`.

### 2.4 — Refnum Types ⭐⭐⭐
**Problem:** Already mentioned in README as "desired next steps." LabVIEW refnums (file, DAQ, VISA, etc.) are opaque references critical for hardware integration.

**Proposal:** Create a generic `LVRefnum<T>` type built on `MagicCookie` with:
- Type-safe wrappers for common refnum categories
- RAII semantics (auto-close on drop when appropriate)
- The "Custom LabVIEW references" mentioned in README as a smart-pointer pattern

### 2.5 — Fixed-Point Numbers ⭐
**Problem:** LabVIEW supports fixed-point (FXP) types for FPGA targets.

**Proposal:** Low priority, but add `LVFixedPoint<const INT_BITS: u8, const FRAC_BITS: u8>` with conversion to/from `f64`. Mainly useful for cRIO/FPGA interop.

---

## 3. Memory Management Enhancements

### 3.1 — Handle Locking API ⭐⭐
**Problem:** LabVIEW's `DSSetHandleSize` documentation warns against locked handles, but there's no lock mechanism exposed. The LabVIEW memory manager supports `DSLockHandle`/`DSUnlockHandle` for pinning memory during operations.

**Proposal:** Wrap `DSLockHandle` and `DSUnlockHandle` with a RAII guard:
```rust
let guard = handle.lock()?;  // Returns HandleGuard<T>
// handle memory is pinned
// guard.deref() gives &T
drop(guard);  // automatically unlocks
```

### 3.2 — Pointer-Based Memory Allocation ⭐⭐
**Problem:** Only handle-based allocation (`DSNewHandle`) is wrapped. LabVIEW also has `DSNewPtr`/`DSNewPClr`/`DSDisposePtr` for non-relocatable memory.

**Proposal:** Add `OwnedUPtr<T>` analogous to `OwnedUHandle<T>`:
- `DSNewPtr(size)` → allocate
- `DSDisposePtr(ptr)` → drop
- Simpler than handles (no double-pointer indirection)

### 3.3 — Zero-Initialized Handle Creation ⭐
**Problem:** `DSNewHandle` creates uninitialized memory. LabVIEW also has `DSNewHClr` for zero-initialized handles.

**Proposal:** Add `OwnedUHandle::new_zeroed(size)` wrapping `DSNewHClr` for safety-critical allocations.

---

## 4. Synchronization & Communication

### 4.1 — Queue Support ⭐⭐⭐
**Problem:** LabVIEW queues are the primary async communication mechanism but aren't supported.

**Proposal:** Wrap `QueueCreate`, `QueueDestroy`, `QueueEnqueue`, `QueueDequeue` functions:
```rust
pub struct LVQueue<T> {
    refnum: MagicCookie,
    _marker: PhantomData<T>,
}

impl<T> LVQueue<T> {
    pub fn enqueue(&self, data: &T) -> Result<()>;
    pub fn try_dequeue(&self, timeout_ms: i32) -> Result<Option<T>>;
}
```

### 4.2 — Notifier Support ⭐⭐
**Problem:** LabVIEW notifiers (single-value broadcast) aren't supported.

**Proposal:** Similar to queue support, wrap `CreateNotifier`, `SendNotification`, `GetNotifierStatus`, `ReleaseNotifier`.

### 4.3 — Typed User Events with Registration ⭐⭐
**Problem:** Current `LVUserEvent<T>::post()` can only send events. There's no way for Rust code to register for or wait on events.

**Proposal:** If LabVIEW exposes event registration APIs, wrap them. Otherwise, document this as a LabVIEW→Rust limitation and suggest queue-based alternatives.

### 4.4 — Semaphore Support ⭐
**Problem:** LabVIEW semaphores aren't represented for cross-language resource locking.

**Proposal:** Wrap LabVIEW semaphore creation/acquisition/release with RAII guard pattern.

---

## 5. Error Handling Improvements

### 5.1 — Custom Error Code Registry ⭐⭐
**Problem:** The crate uses range 542,000–542,999 for internal errors (6 codes defined). Users defining their own library errors need guidance and a mechanism to register custom error ranges.

**Proposal:** Add a `register_error_range(start: i32, end: i32, provider: &str)` utility and a `#[derive(LabVIEWError)]` macro for user error enums:
```rust
#[derive(LabVIEWError)]
#[lv_error_range(543_000)]
pub enum MyLibraryError {
    #[error("Sensor not connected")]
    SensorNotConnected = 543_000,
    #[error("Calibration failed: {0}")]
    CalibrationFailed(String) = 543_001,
}
```

### 5.2 — Panic Catch at FFI Boundary ⭐⭐⭐
**Problem:** If Rust code panics inside an `extern "C"` function, it's undefined behavior. There's no built-in catch mechanism.

**Proposal:** Add a `#[lv_export]` attribute macro or helper that wraps exported functions in `std::panic::catch_unwind`:
```rust
#[lv_export]
pub fn my_function(input: LStrHandle) -> LVStatusCode {
    // panic here → LVStatusCode error instead of UB
}
```
This would:
- Wrap the function body in `catch_unwind`
- Convert panics to `LVStatusCode::PANIC` (new error code)
- Optionally write panic info to an error cluster parameter

### 5.3 — Warning Support in Error Clusters ⭐
**Problem:** `ErrorCluster::set_warning` exists but there's no ergonomic way to accumulate warnings without overwriting error state.

**Proposal:** Add a `WarningAccumulator` that collects warnings and writes them to the error cluster only if no error occurred.

---

## 6. 32-Bit Platform Improvements

### 6.1 — Safe Cluster Field Accessors ⭐⭐⭐
**Problem:** On 32-bit, accessing cluster fields requires unsafe `read_unaligned`/`write_unaligned`. This is error-prone and every field access is unsafe.

**Proposal:** Generate safe accessor methods automatically via derive macro:
```rust
// Generated by #[derive(LabVIEWCluster)]
impl TestStruct {
    pub fn one(&self) -> u8 { unsafe { read_unaligned(addr_of!(self.one)) } }
    pub fn set_one(&mut self, v: u8) { unsafe { write_unaligned(addr_of_mut!(self.one), v) } }
}
```

### 6.2 — 32-Bit NDArray Support ⭐
**Problem:** NDArray integration is 64-bit only due to DST limitations in packed structs.

**Proposal:** Implement 32-bit ndarray views by copying data out of the packed array into an ndarray-owned buffer. Less efficient but functional.

---

## 7. Developer Experience & API Ergonomics

### 7.1 — `#[lv_export]` Function Attribute ⭐⭐⭐
**Problem:** Every exported function requires boilerplate: `#[no_mangle] pub extern "C" fn`, manual error handling, status code conversion.

**Proposal:** Create an attribute macro that handles common patterns:
```rust
#[lv_export]
fn hello_world(mut string: LStrHandle) -> Result<()> {
    string.set(b"Hello World")?;
    Ok(())
}
// Expands to: #[no_mangle] pub extern "C" fn hello_world(...) -> LVStatusCode { ... }
```

Features:
- Auto `#[no_mangle] pub extern "C"`
- Wraps body in panic catch
- Converts `Result<T>` to `LVStatusCode`
- Optionally populates error cluster parameter

### 7.2 — LabVIEW Header Generation ⭐⭐
**Problem:** Users must manually configure Call Library Function Nodes in LabVIEW to match Rust function signatures.

**Proposal:** Add a build-time tool or proc macro that generates a C header file (`.h`) from `#[lv_export]` functions. LabVIEW can import C headers to auto-configure Call Library nodes.

### 7.3 — String Convenience Methods ⭐
**Problem:** String handling requires working with `LStr`/`LStrHandle` through raw byte methods.

**Proposal:** Add convenience methods:
- `LStrHandle::from_str(s: &str)` — encode and set
- `LStr::as_str() -> Result<&str>` — decode and validate UTF-8
- `impl Display for LStr` — already exists, but add `impl From<&str> for LStrOwned`

### 7.4 — Array Builder Pattern ⭐
**Problem:** Creating and populating arrays requires manual resize + index-by-index writes.

**Proposal:** Add a builder:
```rust
let array = LVArrayOwned::<1, f64>::builder()
    .with_dimensions([100])
    .fill_with(|i| i as f64 * 0.1)
    .build()?;
```

---

## 8. Testing & Safety

### 8.1 — Property-Based Testing ⭐⭐
**Problem:** Current tests cover specific cases but may miss edge cases in type conversions.

**Proposal:** Add `proptest` or `quickcheck` tests for:
- Timestamp round-trips (LVTime ↔ epoch ↔ chrono)
- String encoding round-trips (Rust string → LStr → Rust string)
- Array dimension calculations (overflow, negative values)
- Error code conversions

### 8.2 — Miri Testing for Unsafe Code ⭐⭐
**Problem:** The crate contains significant `unsafe` code for FFI, handle dereferencing, and unaligned access.

**Proposal:** Add Miri to CI for catching undefined behavior in unit tests that don't require LabVIEW runtime.

### 8.3 — Fuzzing Targets ⭐
**Problem:** No fuzz testing for data parsing (timestamps from bytes, string decoding).

**Proposal:** Add `cargo-fuzz` targets for `LVTime::from_le_bytes`, `LVTime::from_be_bytes`, `LStr` decoding.

---

## 9. Documentation & Examples

### 9.1 — Comprehensive Integration Guide ⭐⭐⭐
**Problem:** No end-to-end guide for creating a Rust library callable from LabVIEW.

**Proposal:** Create a `docs/` directory with:
- **Getting Started Guide**: Cargo setup, cdylib configuration, basic function export
- **Type Mapping Reference**: Table of LabVIEW types → Rust types with Call Library Node configuration
- **Cluster Guide**: How to define matching structs, 32-bit vs 64-bit caveats
- **Error Handling Guide**: Best practices for error cluster propagation
- **Memory Ownership Guide**: When to use Handle vs Owned vs Ptr

### 9.2 — Example Projects ⭐⭐
**Problem:** The test library exists but isn't structured as user-facing examples.

**Proposal:** Add an `examples/` directory with standalone mini-projects:
- `basic-function` — simplest possible Rust → LabVIEW call
- `data-acquisition-sim` — arrays, timestamps, waveforms
- `async-communication` — user events and occurrences
- `error-handling` — error cluster propagation patterns

### 9.3 — Type Compatibility Matrix ⭐
**Problem:** No clear reference for what works on which platform with which features.

**Proposal:** Add a compatibility matrix to docs:
| Feature | 64-bit Win | 32-bit Win | 64-bit Linux | Feature Flag |
|---------|-----------|-----------|-------------|-------------|
| Basic clusters | ✅ | ⚠️ unaligned | ✅ | — |
| Error clusters | ✅ | ❌ | ✅ | — |
| NDArray | ✅ | ❌ | ✅ | `ndarray` |
| Strings | ✅ | ✅ | ✅ | `link` |
| User events | ✅ | ✅ | ✅ | `sync` |

---

## Priority Summary

| Priority | Item | Impact | Effort |
|----------|------|--------|--------|
| 🔴 High | 5.2 Panic catch at FFI boundary | Safety-critical | Medium |
| 🔴 High | 7.1 `#[lv_export]` attribute macro | Developer experience | High |
| 🔴 High | 1.1 Error clusters on 32-bit | Platform parity | Medium |
| 🔴 High | 2.1 Enum types | Common use case | Low |
| 🔴 High | 9.1 Integration guide | Adoption | Medium |
| 🟡 Medium | 2.4 Refnum types | Hardware integration | High |
| 🟡 Medium | 4.1 Queue support | Async communication | High |
| 🟡 Medium | 1.2 Derive macro for clusters | Ergonomics | High |
| 🟡 Medium | 2.2 Complex numbers | Signal processing | Low |
| 🟡 Medium | 3.1 Handle locking | Safety | Medium |
| 🟡 Medium | 6.1 Safe 32-bit field accessors | Safety | Medium |
| 🟡 Medium | 8.1 Property-based testing | Quality | Medium |
| 🟢 Low | 2.3 Path type | Convenience | Low |
| 🟢 Low | 2.5 Fixed-point numbers | FPGA niche | Medium |
| 🟢 Low | 7.4 Array builder | Convenience | Low |
| 🟢 Low | 8.3 Fuzzing | Security | Low |

---

## Answer: Are Clusters Fully Supported?

**Partially.** Here's the breakdown:

### ✅ What Works
- Custom cluster structs via `labview_layout!` macro on both 32/64-bit
- Clusters containing: numerics, `LStrHandle`, `LVArrayHandle`, `LVTime`, `LVBool`, `LVVariant`, `Waveform`
- Clusters as user event payloads
- Clusters in handles (`UHandle<MyCluster>`) and as pointers (`UPtr<MyCluster>`)
- `ErrorCluster` as a built-in type (64-bit only)

### ⚠️ Limitations
- **32-bit field access requires unsafe code** — every field read/write needs `read_unaligned`/`write_unaligned`
- **Error clusters are 64-bit only** — the `ToLvError` trait and `ErrorClusterPtr` are unavailable on 32-bit
- **No deep clone** — clusters with handle fields (`LStrHandle`, `LVArrayHandle`) cannot be safely deep-copied
- **No compile-time layout validation** — mismatched struct definitions cause silent data corruption
- **Waveform padding is reverse-engineered** — may break with future LabVIEW versions
- **No derive macro** — cluster structs require manual `labview_layout!` wrapping

### ❌ Not Supported
- Automatic generation of matching LabVIEW type definitions
- Nested cluster arrays (array of clusters containing handles)
- String arrays within clusters
- Variant data inspection/creation
