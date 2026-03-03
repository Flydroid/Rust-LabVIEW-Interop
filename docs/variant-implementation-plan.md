# LabVIEW Variant & Type Descriptor Support — Implementation Plan

## Summary

Add a `typedesc` module and extend `LVVariant` in the `labview-interop` crate to support **bidirectional** zero-copy variant data access from Rust. The approach mirrors [h5labview](https://h5labview.sf.net): LabVIEW passes (1) a type descriptor byte string and (2) the Variant handle to the DLL. Rust parses the type descriptor to understand the memory layout and calls the undocumented `LvVariantGetDataPtr` to get a **read/write** data pointer — no flattening of the data payload. This supports large datasets without serialization overhead.

`LvVariantGetDataPtr` returns a mutable pointer. DLL code can both **read from** and **write into** variant data. LabVIEW creates empty typed Variants via `TypeToVariant.vi` (XNodeSupport), and the DLL populates them. This makes variants a practical polymorphic interface — one function signature handles any LabVIEW type, in both directions.

The type descriptor parser is the core deliverable, useful beyond just variants — any CLFN that receives LabVIEW type info can use it.

For **cross-system** messaging (Zenoh with Rust, Python, C++ participants), Protobuf is the recommended wire format. The Variant/typestr approach becomes a **LabVIEW-optimized transport layer** underneath: Protobuf defines the canonical schema, the DLL translates between Protobuf bytes and LabVIEW Variant memory. This gives a three-layer architecture: schema-first cross-system (Protobuf), LV-optimized local (Variant+typestr zero-copy), and pure parsing (typedesc module).

## Background & Prior Art

### LabVIEW Variant Internals

- NI does not document the internal memory layout of a LabVIEW Variant. The `extcode.h` header (LabVIEW 2026) forward-declares `class LvVariant` as opaque with zero public methods.
- No C API exists for creating, reading, writing, or managing Variants from external code.
- DVRs (Data Value References) are also inaccessible — no C API in `extcode.h`.
- However, `LvVariantGetDataPtr` is an **undocumented** LabVIEW runtime export that returns a read/write pointer to the variant's underlying data.
- LabVIEW can create correctly-typed empty Variants from type descriptor bytes using `TypeToVariant.vi` (XNodeSupport). This solves the "return path" problem — the DLL never needs to create Variants, only read/write their data.

### Approaches Considered

| Approach | How It Works | Pros | Cons |
|----------|-------------|------|------|
| **Flatten To String** (Variant++ style) | LabVIEW VI flattens Variant to bytes; DLL parses bytes | Documented, stable, self-contained | Extra copy + serialization; unacceptable for large datasets |
| **Flatten To String** (labview-variant-data) | Python library serializes/deserializes flattened format directly | Complete type coverage incl. Map/Set/Path; version-aware; cross-language | Still requires serialization; Python only |
| **`LvVariantGetDataPtr`** (h5labview style) | Undocumented runtime fn gives raw data pointer; type descriptor passed separately | Zero-copy; direct memory access; proven in production | Undocumented API; could break between LV versions |

### Chosen Approach

**`LvVariantGetDataPtr` + type descriptor** — the h5labview approach. This avoids flattening the data payload entirely. The data pointer is **read/write**, enabling both directions.

The h5labview library at `readwrite.c:78` declares:

```c
// this is a guess at the prototype
TH_REENTRANT EXTERNC void* _FUNCC LvVariantGetDataPtr( void *varhndl );
```

This function takes a Variant handle and returns a mutable pointer to the variant's underlying data.

#### Write INTO Variant (DLL → LabVIEW)

This is proven by h5labview's `H5LVread` — HDF5 data is read directly into a Variant's memory:
1. LabVIEW calls `H5Tquery.vi` → `H5LVget_type()` generates typestr from HDF5 metadata
2. LabVIEW calls `TypeToVariant.vi` (XNodeSupport) → creates an empty Variant of the correct type
3. LabVIEW passes Variant handle + typestr to `H5Dread_var.vi` → CLFN
4. DLL calls `LvVariantGetDataPtr` → gets writable data pointer
5. DLL resizes handles (`NumericArrayResize`/`DSSetHandleSize`) for arrays/strings
6. DLL writes data directly into the Variant's memory

#### Read FROM Variant (LabVIEW → DLL)

Used by h5labview's `H5LVwrite`:
1. LabVIEW: user data → `To Variant` → `Variant To Flattened String` → type cast → typestr
2. LabVIEW passes Variant handle + typestr to CLFN
3. DLL calls `LvVariantGetDataPtr` → reads data directly from Variant memory

### Key h5labview Functions

| Function | File | Purpose |
|----------|------|---------|
| `recurse_typedesc()` | `typeconv.c:222` | Parse typestr → HDF5 type (typestr → layout) |
| `H5LVquery_type()` | `typeconv.c:583` | Generate typestr from HDF5 type (layout → typestr) |
| `H5LVget_type()` | `typeconv.c:863` | Top-level wrapper: opens HDF5 object, calls `H5LVquery_type`, returns typestr |
| `LvVariantGetDataPtr()` | `readwrite.c:78` | Undocumented LabVIEW runtime fn → raw data pointer |
| `get_variant_pointer()` | `typeconv.c:54` | Null-safe wrapper around `LvVariantGetDataPtr` |

`H5LVquery_type` is the **reverse** of `recurse_typedesc` — it builds type descriptor bytes from type metadata. This is the reference implementation for our `TypeDescriptor::to_bytes()` serializer (Step 10).

### labview-variant-data (Python)

The [labview-variant-data](https://github.com/kleinsimon/labview-variant-data) library (MIT, Simon Klein) is a Python implementation that fully reverse-engineers the **flattened** Variant format (`Flatten To String` / `From String`). It works purely with serialized byte streams — no `LvVariantGetDataPtr`. Key findings from this library:

#### Variant Versioning & Type Header LUT

The flattened format has **version headers** that change the binary layout:

- **Version 0** (`0x00000000`): Inline type descriptors — each section embeds its full type header adjacent to its data.
- **Version 0x18008000** (modern, LabVIEW ~2018+): **Type header lookup table (LUT)** — all type descriptors are collected into a table at the top of the stream, and the data section references them by `u16` index. This deduplicates repeated types (e.g., arrays of clusters).

Version 0x18008000 top-level layout:
```
[4B version: 0x18008000]
[4B n_headers: u32]
[N × type headers (same format as type descriptor sections)]
[2B n_data_fields: u16, always 1 for top-level]
[2B type_index → into header LUT]
[data bytes (big-endian)]
[4B n_attributes: u32]
[N × attribute: {u32-len-prefixed name string, recursive variant}]
```

#### Variant Attributes

LabVIEW Variants carry **named attributes** — key-value pairs where keys are strings and values are nested Variants. These are appended after the data payload in the flattened format and are separate from the data pointer returned by `LvVariantGetDataPtr`. Note: `LvVariantGetDataPtr` returns only the data pointer; attributes are a separate concern and are not accessible through it.

#### Additional Type Codes

The library handles several type codes not covered by h5labview:

| Code | Type | Format Details |
|------|------|----------------|
| `0x00` | Void/Nil | No payload; used for empty Variants |
| `0x32` | Path | `PTH0` magic + parts-based serialization |
| `0x73` | Set | `[element_type_header] [u32 count] [elements...]` |
| `0x74` | Map | `[u16 n_types=2] [key_header] [value_header] [u32 n_items] [k₀v₀ k₁v₁ ...]` |

#### Flattened Data is Big-Endian

All data values in the flattened format are **big-endian** (`>i4`, `>f8`, etc.). This is distinct from `LvVariantGetDataPtr` which returns **native-endian** in-memory data. The type descriptor header bytes are always big-endian regardless of access path.

#### Enum Version-Dependent Padding

Enum type headers have version-dependent trailing bytes: version >= `0x08508002` adds 2 bytes of padding in the header offset, while older versions add 1. Member names are pascal strings with `u8` length prefix.

#### Timestamp Format

LabVIEW epoch = **1904-01-01 UTC**. Timestamp = `[i64 seconds since epoch] [u64 fractional (2⁶⁴ scale)]`, 16 bytes total, big-endian.

#### Waveform (Analog Signal) Details

Waveform subcodes map to data types with codes different from the standard numeric type codes:

| Subcode | dtype | Subcode | dtype |
|---------|-------|---------|-------|
| `0x14` | I8 | `0x11` | U8 |
| `0x02` | I16 | `0x12` | U16 |
| `0x15` | I32 | `0x13` | U32 |
| `0x19` | I64 | `0x20` | U64 |
| `0x05` | SGL | `0x03` | DBL |

The analog waveform data layout is: `[timestamp 16B] [dt: f64] [u32 n_elements] [elements...] [error cluster: bool+i32+string] [attributes variant]`.

## Architecture

### Data Flow: Read FROM Variant (LabVIEW → DLL)

```
LabVIEW Variant ──┬── "Variant To Flattened String" → type cast → typestr bytes ──→ CLFN param
                  └── Variant handle (Adapt to Type / pointer) ───────────────────→ CLFN param
                                                                                        │
                                                              ┌─────────────────────────┘
                                                              ▼
                                                    Rust DLL receives:
                                                    1. typestr → parse_type_descriptor() → TypeDescriptor
                                                    2. variant handle → LvVariantGetDataPtr() → *mut data
                                                              │
                                                              ▼
                                                    TypeDescriptor tree tells Rust how to
                                                    READ the raw data pointer
```

### Data Flow: Write INTO Variant (DLL → LabVIEW)

```
┌─ LabVIEW side ─────────────────────────────────────────────────────────────────────┐
│                                                                                    │
│  1. Obtain typestr (from H5LVget_type, Flatten To String, or constant)             │
│  2. TypeToVariant.vi (XNodeSupport) creates empty Variant from typestr             │
│  3. Pass Variant handle + typestr to CLFN                                          │
│                                                                                    │
└──────────────────────────────────┬─────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─ DLL side ─────────────────────────────────────────────────────────────────────────┐
│                                                                                    │
│  1. parse_type_descriptor(typestr) → TypeDescriptor tree                           │
│  2. LvVariantGetDataPtr(handle) → writable *mut data                               │
│  3. For arrays/strings: resize inner handles via MemoryApi                         │
│     (NumericArrayResize / DSSetHandleSize / DSNewHandle)                            │
│  4. Write dimension sizes, then element data (respecting alignment)                │
│                                                                                    │
└──────────────────────────────────┬─────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─ LabVIEW side ─────────────────────────────────────────────────────────────────────┐
│                                                                                    │
│  Populated Variant returned from CLFN                                              │
│  → "Variant To Data" → typed result                                                │
│                                                                                    │
└────────────────────────────────────────────────────────────────────────────────────┘
```

### Type Descriptor Binary Format

Each type descriptor section is structured as:

```
[2 bytes: section length (big-endian)]
[1 byte:  flags (bit 0x40 = has name)]
[1 byte:  type code]
[... type-specific payload ...]
[optional: pascal string name (1 byte length + chars)]
```

For arrays, the outer wrapper contains `[ndims (2B BE)] [0xFFFFFFFF × ndims]` before the element type descriptor.

For clusters, the payload contains `[element count (2B BE)]` followed by N recursive type descriptor sections.

### Type Codes (from h5labview `typeconv.c`)

| Code | Type | Code | Type |
|------|------|------|------|
| `0x01` | I8 | `0x09` | SGL |
| `0x02` | I16 | `0x0A` | DBL |
| `0x03` | I32 | `0x0B` | EXT |
| `0x04` | I64 | `0x0C` | CSG |
| `0x05` | U8 | `0x0D` | CDB |
| `0x06` | U16 | `0x0E` | CEX |
| `0x07` | U32 | `0x15`–`0x17` | Enum (U8/U16/U32) |
| `0x08` | U64 | `0x19`–`0x1E` | Physical Quantity |
| `0x21` | Boolean | `0x30` | String |
| `0x32` | Path | `0x40` | Array |
| `0x50` | Cluster | `0x53` | Variant |
| `0x54` | Waveform | `0x73` | Set |
| `0x74` | Map | `0x00` | Void/Nil |

## Dynamic Sizes: Arrays and Strings

The type descriptor does **not** contain runtime sizes for arrays or strings. It only describes the shape (number of dimensions, element type) and layout. Actual sizes live in the LabVIEW handles at runtime.

### Array Handle Layout

`LvVariantGetDataPtr` returns a handle (pointer-to-pointer) for array/string data:

```
handle → pointer → [ dim₀: u32 ][ dim₁: u32 ]...[ padding ][ element₀ ][ element₁ ]...
                   ╰──── ndims × 4 bytes ────╯   ╰ align  ╯╰──── npts × elem_size ────╯
```

The alignment padding between dimension sizes and element data follows platform rules:
```c
data = (char*)DO_ALIGN(data + 4*ndims, align);  // h5labview readwrite.c:323
```

### String Handle Layout

Strings are LabVIEW `LStr` handles:
```
handle → pointer → [ length: u32 ][ char₀ ][ char₁ ]...
```

### Reading Sizes (Variant → DLL)

Straightforward — dereference the handle and read dimension words:
```c
uint32_t *lvdims = (uint32_t*)**hndl;    // dimension sizes are at start
for (i = 0; i < ndims; ++i)
    dims[i] = lvdims[i];
data = (char*)DO_ALIGN(data + 4*ndims, align);  // skip to element data
```

### Writing Sizes (DLL → Variant)

Requires LabVIEW Memory Manager calls to resize handles:

1. **Numeric arrays**: `NumericArrayResize(tc, ndims, handle, total_bytes)` — uses a type code to ensure correct alignment. For non-numeric arrays (clusters, strings), a "spoof" type code matching the desired alignment is used (see `readwrite.c:285–295`).

2. **Unaligned / packed arrays** (Win32): `DSSetHSzClr(handle, total_bytes + ndims*4)` or `DSNewHClr(total_bytes)`.

3. **Strings inside clusters**: Each string field contains a **string handle** (pointer). The DLL must allocate/resize that inner handle separately via `DSNewHandle`/`DSSetHandleSize`.

4. **Sub-arrays inside clusters**: Same as strings — each sub-array field is a handle that must be independently managed.

This means the write path must **walk the entire TypeDescriptor tree** to find every handle that needs resizing. The layout engine (Step 6) must compute where each handle lives within a cluster's memory.

## Implementation Steps

### Step 1: Add `variant` Feature Flag

**File**: `labview-interop/Cargo.toml`

```toml
[features]
default = ["sync"]
chrono = ["dep:chrono"]
sync = ["link"]
link = ["dep:dlopen2", "dep:dlopen2_derive"]
ndarray = ["dep:ndarray"]
variant = ["link"]  # NEW — enables LvVariantGetDataPtr runtime linkage
```

The `typedesc` module itself has no feature gate — it is pure parsing logic that works without LabVIEW.

### Step 2: Create `typedesc` Module

**New files** in `labview-interop/src/typedesc/`:

| File | Purpose |
|------|---------|
| `mod.rs` | Re-exports |
| `types.rs` | `LvTypeCode` enum and `TypeDescriptor` tree |
| `parser.rs` | Binary parser (port of `recurse_typedesc`) |
| `layout.rs` | Size/alignment computation |

**Register** in `labview-interop/src/lib.rs`:

```rust
pub mod typedesc;
```

### Step 3: Define `LvTypeCode` Enum

**File**: `typedesc/types.rs`

```rust
use num_enum::TryFromPrimitive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[repr(u8)]
pub enum LvTypeCode {
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
    EnumU8 = 0x15,
    EnumU16 = 0x16,
    EnumU32 = 0x17,
    // 0x19–0x1E: physical quantities (base code - 0x10 = numeric code)
    PhysI8 = 0x19,
    PhysI16 = 0x1A,
    PhysI32 = 0x1B,
    PhysI64 = 0x1C,
    PhysU8 = 0x1D,
    PhysU16 = 0x1E,
    Boolean = 0x21,
    String = 0x30,
    Path = 0x32,
    Array = 0x40,
    Cluster = 0x50,
    Variant = 0x53,
    Waveform = 0x54,
    Set = 0x73,
    Map = 0x74,
    Void = 0x00,
}
```

### Step 4: Define `TypeDescriptor` Enum

**File**: `typedesc/types.rs`

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDescriptor {
    Numeric {
        code: LvTypeCode,
        name: Option<String>,
    },
    Boolean {
        name: Option<String>,
    },
    String {
        name: Option<String>,
    },
    Enum {
        base_type: LvTypeCode,
        members: Vec<String>,
        name: Option<String>,
    },
    Array {
        ndims: u16,
        element: Box<TypeDescriptor>,
        name: Option<String>,
    },
    Cluster {
        fields: Vec<TypeDescriptor>,
        name: Option<String>,
    },
    Waveform {
        subcode: u8,
        name: Option<String>,
    },
    Timestamp {
        name: Option<String>,
    },
    PhysicalQuantity {
        base_type: LvTypeCode,
        units: Vec<(u16, i16)>, // (unit_code, power)
        name: Option<String>,
    },
    Path {
        name: Option<String>,
    },
    Map {
        key: Box<TypeDescriptor>,
        value: Box<TypeDescriptor>,
        name: Option<String>,
    },
    Set {
        element: Box<TypeDescriptor>,
        name: Option<String>,
    },
    Void,
}
```

### Step 5: Implement Binary Parser

**File**: `typedesc/parser.rs`

Port logic from h5labview's `recurse_typedesc` (`typeconv.c:222–450`) and `parse_array_td` (`typeconv.c:72–86`).

```rust
/// Internal cursor for parsing type descriptor bytes.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn read_u8(&mut self) -> Result<u8>;
    fn read_u16_be(&mut self) -> Result<u16>;
    fn read_u32_be(&mut self) -> Result<u32>;
    fn read_i16_be(&mut self) -> Result<i16>;
    fn read_pascal_str(&mut self) -> Result<String>;
    fn remaining(&self) -> usize;
}

/// Parse a complete type descriptor from raw bytes.
/// Returns the parsed descriptor and the number of bytes consumed.
pub fn parse(bytes: &[u8]) -> Result<TypeDescriptor, LvInteropError>;

/// Parse type descriptor bytes, detecting if the outermost element is
/// a top-level array wrapper. Returns (Some(ndims), inner_type) for arrays
/// or (None, type) for scalars/clusters.
pub fn parse_with_array(bytes: &[u8]) -> Result<(Option<u16>, TypeDescriptor), LvInteropError>;
```

Key parsing logic per type code:

- **Numerics** (`0x01`–`0x0E`): No payload beyond code. Read optional name.
- **Enums** (`0x15`–`0x17`): `[member_count: u16 BE]` followed by N pascal strings.
- **Physical quantities** (`0x19`–`0x1E`): `[unit_count: u16 BE]` then N × `[unit: u16 BE] [power: i16 BE]`.
- **Boolean** (`0x21`): No payload. Read optional name.
- **String** (`0x30`): Skip 4-byte `0xFFFFFFFF` marker. Read optional name.
- **Array** (`0x40`): `[ndims: u16 BE] [0xFFFFFFFF × ndims]` then recurse for element type.
- **Cluster** (`0x50`): `[element_count: u16 BE]` then recurse N times.
- **Waveform** (`0x54`): `[subcode: u16 BE]`. Subcode 6 = Timestamp (skip cluster contents). Others = numeric waveform.
- **Path** (`0x32`): Skip 4-byte `0xFFFFFFFF` marker. Read optional name. (Data is `PTH0` magic + path parts.)
- **Map** (`0x74`): `[n_types: u16 BE, always 2] [key_type_header] [value_type_header]`. Data: `[n_items: u32 BE] [k₀v₀k₁v₁...]`.
- **Set** (`0x73`): `[element_type_header]`. Data: `[n_items: u32 BE] [elements...]`.
- **Void** (`0x00`): No payload, no data.

No external parsing library needed — the format is simple enough for a hand-rolled cursor with `from_be_bytes()`.

### Step 6: Implement Layout Computation

**File**: `typedesc/layout.rs`

Port alignment rules from h5labview's `LVALIGNMENT` / `DO_ALIGN` macros (`h5labview.h:44–53`).

```rust
/// Returns the LabVIEW memory alignment for the current platform.
pub const fn platform_alignment() -> usize {
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    { 8 }
    #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
    { 1 } // packed on 32-bit Windows
    #[cfg(target_os = "linux")]
    { 4 }
}

/// Align `offset` to the given `alignment`.
pub const fn align_to(offset: usize, alignment: usize) -> usize {
    (offset + alignment - 1) & !(alignment - 1)
}

impl TypeDescriptor {
    /// Returns the in-memory size of this type in bytes.
    pub fn size(&self) -> usize;

    /// Returns the required memory alignment for this type.
    pub fn alignment(&self) -> usize;

    /// For clusters: returns the byte offset of field at `index`,
    /// accounting for platform-specific padding.
    pub fn offset_of(&self, field_index: usize) -> Option<usize>;
}
```

Alignment rules (ported from h5labview):
- Numeric types: alignment = type size (capped at `platform_alignment()`)
- Complex types: alignment = half compound size (CSG aligns to 4, CDB to 8)
- Extended float: depends on `sizeof(floatExt)` — alignment 1 if not a multiple of 4
- Strings: alignment = pointer size
- Arrays (sub-arrays): alignment = pointer size
- Clusters: alignment = max alignment of any field
- After last cluster field: pad to max alignment (struct tail padding)

### Step 7: Add `LvVariantGetDataPtr` to Runtime Linkage

**File**: `labview-interop/src/labview.rs`

```rust
#[cfg(feature = "variant")]
#[derive(WrapperApi)]
pub struct VariantApi {
    /// Extracts a raw data pointer from a LabVIEW Variant handle.
    ///
    /// **WARNING**: This is an undocumented LabVIEW runtime function.
    /// The prototype is reverse-engineered (see h5labview readwrite.c).
    /// It may change or be removed in future LabVIEW versions.
    ///
    /// Returns:
    /// - Valid pointer on success
    /// - NULL for empty variants
    /// - Sentinel value `1` for broken/invalid variants
    #[dlopen2_name = "LvVariantGetDataPtr"]
    variant_get_data_ptr: unsafe extern "C" fn(handle: *mut c_void) -> *mut c_void,
}

#[cfg(feature = "variant")]
static VARIANT_API: LazyLock<Result<Container<VariantApi>>> = LazyLock::new(load_container);

#[cfg(feature = "variant")]
pub fn variant_api() -> Result<&'static Container<VariantApi>> {
    VARIANT_API.as_ref().map_err(|e| e.clone())
}
```

Isolated from `MemoryApi`/`SyncApi` so failure to load doesn't break the rest of the crate.

### Step 8: Extend `LVVariant`

**File**: `labview-interop/src/types/mod.rs`

```rust
/// Represents a LabVIEW Variant. The internal structure is undefined
/// by NI and therefore unavailable.
///
/// With the `variant` feature enabled, the `data_ptr()` method provides
/// zero-copy access to the variant's underlying data via the undocumented
/// `LvVariantGetDataPtr` runtime function. The type descriptor must be
/// obtained separately on the LabVIEW side and parsed via the `typedesc`
/// module to understand the data layout.
#[repr(transparent)]
pub struct LVVariant<'variant>(UHandle<'variant, c_void>);

impl<'variant> LVVariant<'variant> {
    /// Returns true if the variant handle is null.
    pub fn is_empty(&self) -> bool;

    /// Extracts a raw pointer to the variant's underlying data.
    ///
    /// # Safety
    /// - Must only be called when running inside LabVIEW.
    /// - The returned pointer is only valid for the lifetime of the variant.
    /// - The caller must use a parsed TypeDescriptor to correctly interpret
    ///   the memory layout at the returned pointer.
    #[cfg(feature = "variant")]
    pub unsafe fn data_ptr(&self) -> Result<*mut c_void, LvInteropError>;

    /// Interprets the variant's data pointer as a reference to type T.
    ///
    /// # Safety
    /// - T must match the actual LabVIEW type (verified via TypeDescriptor).
    /// - All safety requirements of `data_ptr()` apply.
    #[cfg(feature = "variant")]
    pub unsafe fn as_typed_ref<T>(&self) -> Result<&T, LvInteropError>;
}
```

The `data_ptr()` implementation performs the same validation as h5labview (`readwrite.c:158–168`):
- Null input → `Err(EmptyVariant)`
- Return value `1` (sentinel) → `Err(BrokenVariant)`
- Return value null → `Err(EmptyVariant)`

### Step 9: Add Error Variants

**File**: `labview-interop/src/errors.rs`

Add to the `InternalError` enum:

```rust
#[error("Invalid type descriptor: {0}")]
InvalidTypeDescriptor(String),    // 542,010

#[error("Variant handle is empty or null")]
EmptyVariant,                      // 542,011

#[error("Variant handle is broken (LvVariantGetDataPtr returned sentinel)")]
BrokenVariant,                     // 542,012

#[error("LvVariantGetDataPtr not available in this LabVIEW runtime")]
VariantApiUnavailable,             // 542,013
```

Error codes assigned from the 542,000 range per crate convention.

### Step 10: Type Descriptor Serializer

**File**: `typedesc/parser.rs`

Port logic from h5labview's `H5LVquery_type` (`typeconv.c:583–860`), which is the exact reverse of `recurse_typedesc`.

```rust
impl TypeDescriptor {
    /// Serialize this type descriptor to binary format.
    ///
    /// The output matches the format parsed by `parse()` and can be used:
    /// - As input to LabVIEW's `TypeToVariant.vi` to create empty Variants
    /// - As input to LabVIEW's `Unflatten From String` VI
    /// - As a wire format header for network protocols
    pub fn to_bytes(&self) -> Vec<u8>;
}
```

Serialization rules (mirroring `H5LVquery_type`):
- Each section: `[2B length BE] [1B flags] [1B type code] [payload] [optional pascal-string name]`
- Section length must be even (pad with 0x00 if odd)
- Name flag: set byte at offset+2 to `0x40` when name is present
- **Numerics** (`0x01`–`0x0E`): Just the type code, no payload
- **Strings** (`0x30`): Type code + `0xFFFFFFFF` marker
- **Arrays** (`0x40`): Type code + `[ndims: u16 BE]` + `[0xFFFFFFFF × ndims]` + recursive element
- **Clusters** (`0x50`): Type code + `[count: u16 BE]` + recursive elements (each with name)
- **Physical quantities**: Type code with `0x10` OR'd into flags byte, then `[unit_count: u16 BE]` + `[unit: u16 BE, power: i16 BE]` pairs

This serializer is essential for the write-into-variant path: the DLL (or a consuming crate like lv-zenoh) can generate typestr bytes that LabVIEW uses via `TypeToVariant.vi` to create correctly-typed empty Variants.

## Testing Strategy

### Unit Tests (no LabVIEW required)

Run via `cargo test -- --test-threads=1`.

| Test | What It Verifies |
|------|-----------------|
| Parse scalar U32 | Correct type code, no name, size=4, align=4 |
| Parse scalar with name | Name flag `0x40` correctly extracts pascal string |
| Parse 1D array of DBL | `parse_with_array` returns ndims=1, element=Dbl |
| Parse cluster of {String, I32, Boolean} | Recursive parsing, correct field order |
| Parse enum with 3 members | Member strings extracted correctly |
| Parse nested cluster-of-arrays | Deep recursion works |
| Parse waveform (timestamp vs numeric) | Subcode dispatch |
| Size/alignment: I32 on Win64 | size=4, align=4 |
| Size/alignment: cluster {U8, U32} on Win64 | offset_of(1)=4 (padded), total=8 |
| Size/alignment: cluster {U8, U32} on Win32 | offset_of(1)=1 (packed), total=5 |
| Round-trip: parse → to_bytes → parse | Equality for all supported types |
| Parse Map of {String → I32} | Key/value type headers and nested recursion |
| Parse Set of DBL | Element type header and count |
| Parse Path | `0x32` type code, `0xFFFFFFFF` marker |
| Parse Void | `0x00` type code, empty payload |
| Cross-validate with labview-variant-data | Parse hex bytes from Python test vectors |
| Invalid type code | Returns `Err(InvalidTypeDescriptor)` |
| Truncated input | Returns `Err(InvalidTypeDescriptor)` |

### LabVIEW Integration Tests

Test VIs that:
1. Create Variant containing known data (scalar I32 = 42, 1D DBL array, cluster)
2. Extract type descriptor via `Variant To Flattened String` → type cast to string
3. Call test CLFN passing typestr + Variant handle
4. DLL parses type descriptor, calls `LvVariantGetDataPtr`, reads value
5. DLL returns value/checksum for verification on LabVIEW side

**Large dataset test**: 1M-element DBL array Variant — confirm zero-copy (no flatten overhead, data pointer points into variant's own memory).

**Bidirectional test**: LabVIEW creates empty 1D I32 array Variant via `TypeToVariant.vi`, passes to DLL. DLL resizes array handle to 100 elements, writes values 0..99, returns. LabVIEW reads populated Variant via `Variant To Data` and verifies contents.

## Application: lv-zenoh Polymorphic API

The Variant interface enables a **single set of polymorphic functions** in lv-zenoh that handle any LabVIEW data type, replacing the need for type-specific functions. Combined with Protobuf as the wire format, this gives LabVIEW full interoperability with Rust, Python, and C++ Zenoh participants while preserving zero-copy performance on the LabVIEW side.

### Publish (any type → Zenoh)

```
LabVIEW: data → To Variant → [variant handle + typestr] → zenoh_put_variant()
DLL:     LvVariantGetDataPtr → read LabVIEW memory (guided by TypeDescriptor)
         → populate prost struct → encode Protobuf → zenoh put
Subscribers (Rust/Python/C++): decode Protobuf with generated code
Subscribers (LabVIEW): DLL decodes Protobuf → writes into Variant (see Subscribe below)
```

### Synchronous Get (Zenoh → any type)

```
LabVIEW: typestr → TypeToVariant.vi → empty Variant → zenoh_get_variant()
DLL:     zenoh get → receive Protobuf bytes → prost decode
         → LvVariantGetDataPtr → resize handles → write fields into variant
LabVIEW: populated Variant → Variant To Data → typed result
```

### Subscribe (async, three possible patterns)

| Pattern | How | Tradeoff |
|---------|-----|----------|
| **Sync fetch** | Subscriber queues decoded messages in DLL. LabVIEW polls with pre-allocated Variant: `zenoh_subscriber_recv(sub, variant, typestr)` | Simple, LabVIEW controls pacing. Slight latency. |
| **Raw bytes + UserEvent** | Subscriber posts Protobuf bytes via UserEvent. LabVIEW wrapper calls DLL decode function. | Async, low latency. Decode on LabVIEW thread. |
| **Hybrid** | UserEvent notifies "data available". LabVIEW event handler calls sync fetch with pre-allocated Variant. | Async notification + zero-copy write. |

### Wire Format: Protobuf (Cross-System)

Zenoh is a multi-language system — Rust, Python, C++, and LabVIEW participants must agree on a wire format. Rather than using LabVIEW-specific type descriptor bytes on the wire (which other languages cannot parse without a custom implementation), we use **Protocol Buffers** as the canonical schema-first wire format.

#### Why Protobuf

- Industry standard; mature codegen for Rust (`prost`), Python, C++, and many others
- Zenoh supports `zenoh::Encoding::APPLICATION_PROTOBUF` natively
- `.proto` files are the single source of truth for data types across all participants
- Compact binary encoding with backward/forward compatibility
- Schema evolution (adding fields) without breaking existing subscribers

#### Type Mapping: Protobuf → LabVIEW → Rust

| Protobuf | LabVIEW | Rust |
|----------|---------|------|
| `double` | DBL | `f64` |
| `float` | SGL | `f32` |
| `int32`/`sint32` | I32 | `i32` |
| `int64`/`sint64` | I64 | `i64` |
| `uint32` | U32 | `u32` |
| `uint64` | U64 | `u64` |
| `bool` | Boolean | `bool` |
| `string` | String | `String` |
| `bytes` | U8 Array | `Vec<u8>` / `Bytes` |
| `repeated T` | 1D Array of T | `Vec<T>` |
| `message` (nested) | Cluster | `struct` |
| `enum` | Enum (Ring) | `enum` |
| `google.protobuf.Timestamp` | Timestamp (DBL or cluster) | `prost_types::Timestamp` |
| `oneof` | Variant (with type indicator) | Rust enum |

#### Three-Layer Architecture

```
┌─ Layer 1: Cross-System (Protobuf) ─────────────────────────────────────────────┐
│                                                                                │
│  .proto schema → prost codegen (Rust) / protoc (Python/C++)                    │
│  Wire format: Protobuf binary on Zenoh payloads                                │
│  Encoding: zenoh::Encoding::APPLICATION_PROTOBUF                               │
│                                                                                │
└──────────────────────────────────┬─────────────────────────────────────────────┘
                                   │
                                   ▼
┌─ Layer 2: LV-Optimized (Variant + typestr) ────────────────────────────────────┐
│                                                                                │
│  lv-zenoh DLL translates between Protobuf bytes ↔ LabVIEW Variant memory.      │
│  Uses LvVariantGetDataPtr for zero-copy access to Variant data.                │
│  TypeDescriptor drives the field-by-field translation.                          │
│  LabVIEW never sees Protobuf bytes — only native typed Variants.               │
│                                                                                │
└──────────────────────────────────┬─────────────────────────────────────────────┘
                                   │
                                   ▼
┌─ Layer 3: Pure Parsing (typedesc module) ──────────────────────────────────────┐
│                                                                                │
│  labview-interop typedesc module: parse/serialize type descriptors.             │
│  Compute LabVIEW memory layout (size, alignment, offsets).                     │
│  No Zenoh dependency, no Protobuf dependency — reusable by any crate.          │
│                                                                                │
└────────────────────────────────────────────────────────────────────────────────┘
```

#### Data Flow: Publish (LabVIEW → Zenoh → any subscriber)

```
LabVIEW: data → To Variant → [variant handle + typestr] → zenoh_put_variant()
DLL:     LvVariantGetDataPtr → read LabVIEW memory (guided by TypeDescriptor)
         → populate prost struct fields → encode to Protobuf bytes → zenoh put
Zenoh:   Protobuf payload delivered to Rust/Python/C++/LabVIEW subscribers
```

#### Data Flow: Subscribe (Zenoh → LabVIEW)

```
Zenoh:   Protobuf payload received by lv-zenoh subscriber task
DLL:     prost::Message::decode() → iterate struct fields
         → write into LabVIEW Variant memory (guided by TypeDescriptor)
         → resize handles for arrays/strings as needed
LabVIEW: populated Variant → Variant To Data → typed result
```

#### LabVIEW-Side Schema Workflow

LabVIEW doesn't have native Protobuf support, so the DLL handles all serialization. The workflow:

1. **Define schema**: Create `.proto` files (shared across all system participants)
2. **Generate LabVIEW types**: A build-time tool generates `.ctl` typedef files from `.proto` — these define the LabVIEW clusters matching each Protobuf message
3. **Generate typestr constants**: Same tool (or a LabVIEW utility) produces the type descriptor bytes for each generated `.ctl`
4. **At runtime**: LabVIEW passes Variant + typestr to CLFN. The DLL maps fields between Protobuf and LabVIEW memory layouts.

The `.proto → .ctl` generator is a separate tool (future work). Initially, users can manually create matching `.ctl` files and use `Variant To Flattened String` to extract typestr.

#### Fallback: LabVIEW-to-LabVIEW Only

For simple deployments where all endpoints are LabVIEW, the raw LabVIEW flattened format (typestr + data bytes) can be used directly as the wire format. The type descriptor parser in `labview-interop` serves as the schema. This avoids the Protobuf dependency but is not interoperable with non-LabVIEW participants.

## Key Decisions

| Decision | Rationale |
|----------|----------|
| Protobuf as cross-system wire format | Industry standard schema-first format; `.proto` is single source of truth for all Zenoh participants (Rust, Python, C++, LabVIEW) |
| Variant/typestr as LabVIEW-optimized layer | Zero-copy LabVIEW memory access underneath the Protobuf translation — LabVIEW never handles raw Protobuf bytes |
| Three-layer architecture | Separates concerns: cross-system interop (Protobuf), LabVIEW performance (Variant), reusable parsing (typedesc) |
| `LvVariantGetDataPtr` behind `variant` feature flag | Undocumented API; opt-in acknowledges the risk || Bidirectional (read/write) variant access | h5labview proves both directions work: `H5LVwrite` reads from variants, `H5LVread` writes into them |
| `TypeToVariant.vi` for Variant creation | LabVIEW creates Variants from typestr bytes — DLL never needs to create Variants, only access their data |
| `H5LVquery_type` as serializer reference | The exact reverse of `recurse_typedesc`; proven in production for generating valid typestr bytes |
| Dynamic sizes in handles, not type descriptors | Array dimensions and string lengths live in the handle data at runtime. The write path must resize handles via Memory Manager. || No `nom` dependency | Hand-rolled cursor with `from_be_bytes()` — format is simple, zero added deps |
| Type descriptor is LabVIEW-side responsibility | LabVIEW VIs extract and pass the type descriptor bytes; Rust does not extract type info from the Variant handle itself |
| Platform-aware alignment via `cfg` attributes | Matches LabVIEW's actual memory layout rules per platform |
| `VariantApi` as separate `WrapperApi` struct | Isolated from `MemoryApi`/`SyncApi` so failure to load doesn't break the crate |
| DVRs explicitly excluded | No C API in LabVIEW 2026 `extcode.h`; no path to implement from external code |
| Error codes in 542,000 range | Matches existing crate convention |
| Map, Set, Path, Void type codes added | Discovered via labview-variant-data; needed for complete type descriptor coverage |
| Flattened format versioning noted but deferred | Version 0 vs 0x18008000 LUT format matters only for flattened-stream parsing, not for typestr-only parsing |

## References

- **h5labview** `typeconv.c:222` — `recurse_typedesc` function (typestr → type layout, the parser reference)
- **h5labview** `typeconv.c:583` — `H5LVquery_type` function (type layout → typestr, the serializer reference)
- **h5labview** `typeconv.c:863` — `H5LVget_type` function (top-level: opens HDF5 object → calls `H5LVquery_type` → returns typestr)
- **h5labview** `typeconv.c:54` — `get_variant_pointer` (null-safe `LvVariantGetDataPtr` wrapper)
- **h5labview** `readwrite.c:78` — `LvVariantGetDataPtr` declaration (undocumented LabVIEW runtime fn)
- **h5labview** `readwrite.c:156–170` — Reading FROM variant: `LvVariantGetDataPtr` + sentinel validation
- **h5labview** `readwrite.c:280–330` — Writing INTO variant: handle resizing (`NumericArrayResize`, `DSSetHSzClr`, `DSNewHClr`), dimension writing, alignment
- **h5labview** `readwrite.c:406–424` — Writing FROM variant: `LvVariantGetDataPtr` for `H5LVwrite`
- **h5labview** `h5labview.h` — `LVALIGNMENT`, `DO_ALIGN` macros, LabVIEW type structs
- **h5labview** `H5Tquery.vi` + `TypeToVariant.vi` (XNodeSupport) — LabVIEW-side flow: generate typestr → create empty Variant
- **VariantLabVIEWpp** — C++ `std::variant` approach using flattened strings (rejected for large data)
- **labview-variant-data** — Python library (MIT, Simon Klein) that fully reverse-engineers the LabVIEW Variant flattened format
- **LabVIEW 2026** `extcode.h` — Confirmed no Variant C API, no DVR API
- **labview-interop** existing patterns — `UHandle`, `LVVariant` (opaque placeholder), `labview_layout!`, `WrapperApi` linkage, `MemoryApi` (`NumericArrayResize`, `DSNewHandle`, `DSSetHandleSize`)
- **Protocol Buffers** — [protobuf.dev](https://protobuf.dev/) — wire format specification, language guide
- **prost** — [docs.rs/prost](https://docs.rs/prost/) — Rust Protobuf implementation; `prost-build` for `.proto` → Rust codegen
- **Zenoh Protobuf encoding** — `zenoh::Encoding::APPLICATION_PROTOBUF`
- **labview-variant-data** — [github.com/kleinsimon/labview-variant-data](https://github.com/kleinsimon/labview-variant-data) — Python library (MIT) that fully reverse-engineers the LabVIEW Variant flattened format. Covers versioning (v0 and v0x18008000), type header LUT, variant attributes, Map (`0x74`), Set (`0x73`), Path (`0x32`), Void (`0x00`), Enum version-dependent padding, Timestamp (1904 epoch, i64+u64), analog waveform subcodes. Useful as cross-validation reference and for understanding the flattened stream structure.
- **NI Knowledge Base** — [kA00Z0000015CmaSAE](https://knowledge.ni.com/KnowledgeArticleDetails?id=kA00Z0000015CmaSAE&l=en-US) — Information on LabVIEW flattened data format versioning
## Future Work

### `.proto` → LabVIEW `.ctl` Generator

A build-time tool that reads `.proto` files and generates:
- LabVIEW `.ctl` typedef files (clusters matching each Protobuf message)
- Type descriptor byte constants (typestr for each `.ctl`)
- Documentation mapping Protobuf field numbers to LabVIEW cluster field indices

This could be a standalone Rust CLI tool or a `build.rs` integration. Until this exists, users manually create matching `.ctl` files.

### Rust Struct Codegen from Type Descriptors

Three levels of Rust codegen from LabVIEW type descriptors, useful for Rust applications that need to work with LabVIEW-originated data:

1. **Runtime dynamic** (Step 6 layout engine): Walk `TypeDescriptor` tree at runtime, read/write fields by offset. No codegen needed. Works for any type but no compile-time type safety.

2. **Build-time codegen**: A tool reads typestr bytes and generates `#[repr(C)]` Rust structs with correct field order, types, and padding. Runs at build time (via `build.rs` or CLI). Output:
   ```rust
   // Generated from LabVIEW cluster {timestamp: DBL, channel: String, values: 1D Array of I32}
   #[repr(C)]
   struct Measurement {
       timestamp: f64,
       channel: LStrHandle,
       values: LVArrayHandle<i32, 1>,
   }
   ```

3. **Derive macro** (most advanced): `#[derive(LabViewLayout)]` attribute macro that validates a user-written Rust struct against a type descriptor at compile time, ensuring field order, sizes, and alignment match the LabVIEW layout.

These are independent of the Protobuf wire format — they address the case where Rust code needs to directly interpret LabVIEW memory layouts (e.g., inside the DLL's Protobuf↔Variant translation layer).