# Variant API C Test DLL

A standalone C DLL for probing undocumented LabVIEW Variant API functions at runtime. This was used during development of the `labview-interop` crate's variant support to reverse-engineer the calling conventions and return values of internal LabVIEW functions before committing to a Rust implementation.

## Purpose

LabVIEW's variant internals are completely undocumented. The `extcode.h` header declares `class LvVariant` as opaque with zero public methods. However, the LabVIEW runtime (`labview.exe` / `lvrt.dll`) exports several `LvVariant*` functions that can be resolved at runtime via `GetProcAddress`.

This C DLL was written to:

1. **Discover** which variant functions exist and are callable
2. **Determine calling conventions** — does each function expect the handle (`T**`) or the dereferenced inner pointer (`T*`)?
3. **Dump raw bytes** from type descriptors and data pointers to understand memory layout
4. **Validate assumptions** with SEH (`__try/__except`) crash protection, so a bad guess doesn't take down LabVIEW

All output is appended to `C:\Temp\variant_c_test.log`.

## Exported Functions

### `test_variant_c_read_i32(variant, result) → i32`

Reads an I32 value from a variant by testing two strategies for calling `LvVariantGetDataPtr`:

- **Try 1**: Pass the handle directly (the raw `void*` from LabVIEW)
- **Try 2**: Dereference the handle once, then pass the inner pointer

This determined that the correct calling convention is **Try 2** — dereference the `UHandle` once before calling `LvVariantGetDataPtr`, matching [h5labview](https://h5labview.sf.net)'s approach.

**CLFN setup:**
| Parameter | Type | Passing |
|-----------|------|---------|
| variant | Variant | Adapt to Type, Handles by Value |
| result | I32 | Pointer to Value |
| return | I32 | — |

### `probe_variant_typedesc(variant) → i32`

Extracts and dumps the type descriptor from any variant via `GetTypeFromLvVariant`. Interprets the raw bytes as:

- Type descriptor size (first 2 bytes, LE u16)
- Type code (next 2 bytes, LE u16)
- Full hex dump of all descriptor bytes
- Human-readable type name (I32, DBL, String, Cluster, Array, etc.)
- For clusters: element count and nested element type codes
- For arrays: dimension count and element type

Test with different variant types (I32, DBL, Bool, String, Cluster, Array) to see the descriptor format for each.

**CLFN setup:**
| Parameter | Type | Passing |
|-----------|------|---------|
| variant | Variant | Adapt to Type, Handles by Value |
| return | I32 | — |

### `probe_variant_api(variant) → i32`

Comprehensive probe of all known `LvVariant*` exports. Tests each function with both the handle and the dereferenced inner pointer, wrapped in SEH crash protection. Functions probed:

| Function | What it returns |
|----------|----------------|
| `LvVariantIsEmpty` | `i32` — 0 = has data, non-zero = empty |
| `LvVariantGetCompleteDataSize` | `size_t` — total data size in bytes |
| `LvVariantGetType` | `void*` — pointer to type descriptor (BE format) |
| `GetTypeFromLvVariant` | `void*` — pointer to type descriptor (native-endian format) |
| `GetVariantPtrIfValid` | `void*` — validated variant pointer |
| `LvVariantGetContents` | `void*` — pointer to variant contents |
| `LvVariantGetDataPtr` | `void*` — pointer to raw data (known working reference) |

**CLFN setup:**
| Parameter | Type | Passing |
|-----------|------|---------|
| variant | Variant | Adapt to Type, Handles by Value |
| return | I32 | — |

## Runtime Symbol Resolution

The DLL does **not** link against any LabVIEW SDK headers or libraries. All functions are resolved at runtime via `GetProcAddress`, searching in order:

1. The current process (`GetModuleHandle(NULL)`) — covers `labview.exe`
2. `labview` module — the LabVIEW IDE
3. `lvrt` module — the LabVIEW runtime engine

This means the DLL can be built without the LabVIEW SDK installed.

## Building

Requires the MSVC C compiler (`cl.exe`). Open a **VS Developer Command Prompt** (or use `vcvarsall.bat`) and run:

```cmd
cd labview-test-project\Variant Tests
cl /LD variant_test.c /Fe:variant_test.dll
```

This produces `variant_test.dll` (and `variant_test.lib`, `variant_test.exp`, `variant_test.obj`).

No external dependencies are needed — only Windows SDK headers (`stdio.h`, `stdint.h`, `windows.h`).

## Usage

1. Build the DLL as described above
2. Create `C:\Temp\` if it doesn't exist (log output directory)
3. In LabVIEW, create a VI with a **Call Library Function Node** pointing to `variant_test.dll`
4. Configure parameters as shown in the CLFN tables above
5. Wire a Variant (e.g., from **Variant to Data** or **To Variant**) to the variant input
6. Run the VI
7. Check `C:\Temp\variant_c_test.log` for results

Test VIs are provided alongside this file: `test.vi` (Rust DLL tests) and `test_c.vi` (C DLL tests).

## Key Findings

The probing established these facts, now encoded in `labview-interop`:

- **Handle convention**: LabVIEW passes variants as `UHandle` (`T**`). With "Handles by Value" CLFN config, the parameter is the handle value itself. All `LvVariant*` functions expect the **dereferenced** inner pointer (`*handle`), not the handle.
- **`LvVariantGetDataPtr`**: Returns a `*mut c_void` pointing directly to the variant's data. Returns `NULL` for empty variants, sentinel value `1` for broken/invalid variants.
- **`GetTypeFromLvVariant`**: Returns a pointer to the type descriptor bytes in **native endian** (little-endian on x86/x64). Format: `[u16 size][u16 code][payload...]`.
- **`LvVariantGetType`**: Similar but returns type descriptor in **big-endian** format (matching LabVIEW's "Flatten To String" convention).
- **`LvVariantIsEmpty`**: Returns 0 for non-empty, non-zero for empty variants. Expects the **dereferenced** inner pointer.

## Status

This C DLL is a **development/research tool**, not production code. Now that the variant API behavior is understood and encoded in Rust (`labview-interop`'s `VariantApi` and `LVVariant`), this file serves as historical documentation of the reverse-engineering process.
