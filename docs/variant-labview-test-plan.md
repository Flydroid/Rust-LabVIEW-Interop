# LabVIEW Variant Test Plan — Integrated Unit Tests

Companion to [variant-implementation-plan.md](variant-implementation-plan.md).
This is the build sheet for the LabVIEW side: which VIs to create, the exact
CLFN definitions, the test case inventory with expected results, and how it
all runs from the command line via g-cli.

**Design invariant:** a new test type never needs a new Rust export or a new
CLFN configuration — the generic oracle export handles every type. VI count
is unconstrained; prefer many small, single-purpose VIs (idiomatic LabVIEW)
over one monolithic case diagram.

---

## 1. How it integrates with the existing harness

The project already runs JKI VI Tester through g-cli (justfile):

```
just integration-tests-x64              # LabVIEW 2020 (default)
just lv_ver=2025 integration-tests-x64  # LabVIEW 2025
```

This builds `labview-test-library` (the Rust DLL) and executes every test
class registered in `labview-test-project/rust-interop-test.lvproj`,
producing `x64.xml`. **The only wiring needed for command-line execution is
registering the new `Variant Tests.lvclass` in the lvproj** — then the
existing targets pick it up. Save all VIs in the LabVIEW version you will
run under g-cli.

`Library Path.vi` (project root) resolves the built DLL path — reuse it for
every CLFN, as the existing test classes do.

---

## 2. CLFN definitions

Common settings for all nodes:

- **Library path**: wire from `Library Path.vi` (path input terminal)
- **Calling convention**: C
- **Thread**: for first bring-up use *Run in UI thread*; once green, switch
  to *Run in any thread* and re-run. The memory manager calls are
  thread-safe, but the `LvVariant*` exports are undocumented — verify
  empirically before trusting reentrant execution.

### 2.1 `variant_to_canonical_string` — the oracle (primary node)

`int32_t variant_to_canonical_string(variant, result)`

| Parameter | CLFN Type | Configuration |
|-----------|-----------|---------------|
| `variant` | Adapt to Type | **Handles by Value** |
| `result` | String | **String Handle** |
| return | Numeric | Signed 32-bit Integer |

Wire an empty string constant into `result`; the DLL resizes the handle and
writes the canonical string (format spec: `typedesc/value.rs` module docs,
summarized in §5).

### 2.2 `variant_typedesc_hex` — golden-vector capture

`int32_t variant_typedesc_hex(variant, result)`

Identical configuration to 2.1. Returns the raw `GetTypeFromLvVariant`
bytes as a lowercase hex string (no separators).

### 2.3 `variant_dbl_array_sum` — zero-copy large-array check

`int32_t variant_dbl_array_sum(variant, sum, len)`

| Parameter | CLFN Type | Configuration |
|-----------|-----------|---------------|
| `variant` | Adapt to Type | Handles by Value |
| `sum` | Numeric | 8-byte Double, **Pointer to Value** |
| `len` | Numeric | Unsigned 64-bit Integer, **Pointer to Value** |
| return | Numeric | Signed 32-bit Integer |

### 2.4 Legacy scalar probes (optional smoke test)

`test_variant_read_i32(variant, *i32)`, `test_variant_read_dbl(variant,
*f64)`, `test_variant_read_bool(variant, *u8)`,
`test_variant_write_i32(variant, i32)`, `test_variant_read_string(variant,
String Handle)` — variant always *Adapt to Type / Handles by Value*, out
params *Pointer to Value*. These predate the oracle; keep them in one
smoke-test method or drop them.

### 2.5 Status codes (return value)

| Code | Meaning |
|------|---------|
| 0 | success |
| 542007 | invalid/unparseable type descriptor (**capture the hex when you see this** — parser gap) |
| 542008 | empty variant / null handle |
| 542009 | broken variant (sentinel) or corrupt data (negative length/dim) |
| 542010 | variant API not available in this runtime |
| 542011 | variant type mismatch (wrong type wired to `variant_dbl_array_sum`) |
| 542012 | data pointer misaligned for requested type |
| 542099 | Rust panic caught at the FFI boundary (always a bug — report it) |

---

## 3. VIs to create

### 3.1 Typedef

**`Variant Test Case.ctl`** — cluster:

| Field | Type | Notes |
|-------|------|-------|
| `name` | String | unique case id, goes in failure reports and the golden TSV |
| `data` | Variant | the value under test |
| `expected` | String | expected canonical string; the single value `*` means "don't compare the string, status only" (for run-variable outputs like refnum cookies) |
| `expected status` | I32 | usually 0; non-zero for deliberate error cases |

### 3.2 Case VIs (one per category — grow freely)

Each is a plain VI, no inputs, one output: 1-D array of `Variant Test Case`.
Small diagrams: constant → `To Variant` → Bundle → Build Array. Adding a
case or a whole new category VI later is cheap; there is no requirement to
centralize.

- `Cases Scalars.vi`
- `Cases Strings.vi`
- `Cases Arrays.vi`
- `Cases Clusters.vi`
- `Cases Nested.vi`
- `Cases Special.vi`

**`Build All Cases.vi`** — concatenates the outputs of all case VIs
(used by the capture VI; test methods call their own category VI directly).

### 3.3 Engine subVIs

**`Check Cases.vi`** — in: case array; out: `all passed` (bool),
`report` (string). For each case: call the §2.1 CLFN → compare return
status against `expected status` and (unless `expected` = `*`) the output
string against `expected` → append failures to the report as
`name | expected <e> | got <g> | status <s>`.

**`Bytes To Hex.vi`** — in: string (raw bytes); out: lowercase hex string.
String to Byte Array → For loop `Format Into String "%02x"` → Concatenate.
Needed by the capture VI.

### 3.4 The VI Tester class

**`Variant Tests.lvclass`** in `labview-test-project/Variant Tests/`,
copying the `String Tests` pattern (`setUp.vi`, `tearDown.vi`,
`testExample.vit`). Register it in `rust-interop-test.lvproj`. Test
methods (each: category case VI → `Check Cases.vi` → VI Tester
assert-true with the report as the failure message):

| Method | Cases | Purpose |
|--------|-------|---------|
| `test Variant Scalars.vi` | Cases Scalars | all integer/float/bool widths |
| `test Variant Strings.vi` | Cases Strings | content, empty, escaping |
| `test Variant Arrays.vi` | Cases Arrays | 1-D/2-D/3-D, empty, element kinds |
| `test Variant Clusters.vi` | Cases Clusters | flat + padding torture |
| `test Variant Nested.vi` | Cases Nested | nesting, handles-in-clusters, cluster arrays |
| `test Variant Special.vi` | Cases Special | Tier-3 `unsupported(...)`, error paths |
| `test Large Array Zero Copy.vi` | (direct) | §6 |
| `test Variant Scalar Probes.vi` | (direct, optional) | legacy §2.4 exports |

### 3.5 Capture VI (standalone, not a test)

**`Capture Golden Vectors.vi`** — `Build All Cases.vi` → for each case:

1. §2.2 CLFN → native descriptor hex
2. `Variant To Flattened String` → *type string* output (I16 array) →
   `Flatten To String` (big-endian, **no** length prepend) →
   `Bytes To Hex.vi` → flattened typestr hex
3. Append the TSV line `name<TAB>native_hex<TAB>flattened_hex<TAB>expected`

Output file: `labview-interop/tests/golden/captured.tsv` (derive the repo
root from *Current VI's Path*). Afterwards run `cargo test -p
labview-interop --test golden_typedesc` and **commit the TSV** — one capture
run becomes permanent LabVIEW-free CI coverage.

---

## 4. Torture cluster typedefs (`.ctl`)

Field **labels matter**: the canonical string prints them
(`{a=1,b=2}`), so labels must match this table exactly (case-sensitive),
and every field needs a visible non-empty label.

| Ctl | Fields (label: type) | Proves |
|-----|----------------------|--------|
| `PadA.ctl` | `a`: U8, `b`: U64 | classic 7-byte pad |
| `PadB.ctl` | `a`: U8, `b`: U16, `c`: U8, `d`: U32, `e`: U8, `f`: U64 | every alignment class |
| `Tail.ctl` | `a`: U64, `b`: U8 | tail padding; test **as a 3-element array** (stride 16, not 9) |
| `NestedMid.ctl` | `a`: U8, `inner`: cluster{`x`: U8, `y`: U32}, `b`: U8 | nested-cluster alignment |
| `HandleMix.ctl` | `f1`: Bool, `name`: String, `f2`: Bool, `data`: 1-D DBL array, `f3`: Bool | handles interleaved with 1-byte fields |
| `Deep.ctl` | `a`: cluster{`b`: cluster{`c`: cluster{`v`: I32}}} | recursion depth |
| `TimeCluster.ctl` | `tag`: U8, `t`: Timestamp | **decides the timestamp-alignment question** (§7) |
| `ArrInClus.ctl` | `id`: U16, `values`: 1-D I32 array, `mat`: 2-D DBL array | arrays inside clusters |

---

## 5. Case inventory with expected canonical strings

Canonical format rules that matter when authoring expecteds: integers plain
decimal; floats via Rust `Display` (use exactly-representable values — `3.5`
not `3.14`); bools `true`/`false`; strings double-quoted with `"`/`\`
escaped and non-printable bytes as `\xNN` lowercase; 1-D arrays `[a,b,c]`;
N-D arrays `dims[d0,d1]:[flat row-major]`; clusters `{label=value,…}`;
timestamps `ts(seconds,fractions)` (seconds since 1904-01-01 **UTC**).
Keep test strings ASCII.

### Cases Scalars

| name | LabVIEW constant | expected |
|------|------------------|----------|
| `i8_neg` | I8 = -7 | `-7` |
| `u8_max_range` | U8 = 200 | `200` |
| `i16` | I16 = -12345 | `-12345` |
| `u16` | U16 = 54321 | `54321` |
| `i32` | I32 = -100000 | `-100000` |
| `u32` | U32 = 4000000000 | `4000000000` |
| `i64` | I64 = -5000000000 | `-5000000000` |
| `u64` | U64 = 10000000000000000000 | `10000000000000000000` |
| `f32` | SGL = 3.5 | `3.5` |
| `f64_neg` | DBL = -0.25 | `-0.25` |
| `f64_zero` | DBL = 0 | `0` |
| `bool_true` | TRUE | `true` |
| `bool_false` | FALSE | `false` |

### Cases Strings

| name | constant | expected |
|------|----------|----------|
| `str_simple` | `Hello Variant` | `"Hello Variant"` |
| `str_empty` | empty string | `""` |
| `str_quote` | `say "hi"` | `"say \"hi\""` |
| `str_newline` | `a` + LF + `b` (\-codes display) | `"a\x0ab"` |

### Cases Arrays

| name | constant | expected |
|------|----------|----------|
| `arr_i32` | 1-D I32 `[10,-20,30]` | `[10,-20,30]` |
| `arr_f64` | 1-D DBL `[1.5,2.5]` | `[1.5,2.5]` |
| `arr_u8` | 1-D U8 `[0,255]` | `[0,255]` |
| `arr_empty` | 1-D DBL, 0 elements | `[]` |
| `arr_2d` | 2-D I32, 2×3 rows `[1,2,3]`,`[4,5,6]` | `dims[2,3]:[1,2,3,4,5,6]` |
| `arr_3d` | 3-D U8, 2×2×2 = 1..8 | `dims[2,2,2]:[1,2,3,4,5,6,7,8]` |
| `arr_str` | 1-D String `["ab","c"]` | `["ab","c"]` |

### Cases Clusters

Sentinel values chosen so any wrong offset produces a visibly wrong number.

| name | constant | expected |
|------|----------|----------|
| `pad_a` | PadA{a=1, b=1000000} | `{a=1,b=1000000}` |
| `pad_b` | PadB{1,2,3,4,5,6} | `{a=1,b=2,c=3,d=4,e=5,f=6}` |
| `nested_mid` | NestedMid{a=1, inner{x=2,y=300}, b=4} | `{a=1,inner={x=2,y=300},b=4}` |
| `deep` | Deep{a{b{c{v=42}}}} | `{a={b={c={v=42}}}}` |
| `time_cluster` | TimeCluster{tag=7, t=2020-01-01 00:00:00 **UTC**} | `{tag=7,t=ts(3660681600,0)}` |

### Cases Nested

| name | constant | expected |
|------|----------|----------|
| `handle_mix` | HandleMix{true, "abc", false, [1.5,2.5,3.5], true} | `{f1=true,name="abc",f2=false,data=[1.5,2.5,3.5],f3=true}` |
| `arr_in_clus` | ArrInClus{id=9, values=[1,2,3], mat=2×2 [1.5,2.5],[3.5,4.5]} | `{id=9,values=[1,2,3],mat=dims[2,2]:[1.5,2.5,3.5,4.5]}` |
| `arr_of_clus` | 1-D array of Tail: {11,1},{22,2},{33,3} | `[{a=11,b=1},{a=22,b=2},{a=33,b=3}]` |
| `empty_str_in_clus` | HandleMix{false, "", false, empty array, false} | `{f1=false,name="",f2=false,data=[],f3=false}` |

### Cases Special (Tier 2/3 + error paths)

| name | constant | expected | expected status |
|------|----------|----------|-----------------|
| `enum_u16` | Enum U16 {Voltage,Current,Resistance} = Resistance | `enum(2:Resistance)` | 0 |
| `enum_u8` | Enum U8 {A,B} = B | `enum(1:B)` | 0 |
| `timestamp` | Timestamp 2020-01-01 00:00:00 UTC | `ts(3660681600,0)` | 0 |
| `complex_cdb` | CDB = 1+2i | `unsupported(CDB)` | 0 |
| `path` | Path `C:\Temp` | `unsupported(Path)` | 0 |
| `map` | Map<String,I32> {"a":1} | `unsupported(Map)` | 0 |
| `set` | Set<I32> {1,2} | `unsupported(Set)` | 0 |
| `waveform` | DBL waveform, 3 samples | `unsupported(Waveform)` | 0 |
| `variant_in_clus` | cluster{`v`: variant of I32 5} | `{v=unsupported(Variant)}` | 0 |
| `empty_variant` | fresh variant constant (no data) | `*` | **probe** — record actual (likely 542008 or `void`), then lock in |
| `edvr_refnum` | To Variant of EDVR refnum (needs the EDVR example plugin) | `*` (cookie varies per run) | 0 |

Enum note: the flattened-format research says enum headers have
version-dependent padding — the parser may fail here (status 542007). If it
does, that is a *finding*, not a broken test: capture the hex, file it, fix
the parser, re-run.

---

## 6. `test Large Array Zero Copy.vi`

1. Ramp 0…999999 (1M DBL) → `To Variant`.
2. §2.3 CLFN. Expected: status 0, `len` = 1000000, `sum` = **499999500000**
   (exact in f64 — integer partial sums stay below 2^53).
3. Wrap the call in Tick Count (ms) before/after and include the elapsed
   time in the report (don't assert on it — timing asserts are flaky; the
   correctness assert plus an eyeballed ~sub-millisecond time is the
   zero-copy evidence).
4. Also feed the same variant to the §2.1 oracle **only if curious** — it
   will work but allocates the full `LvValue` tree; that's the documented
   non-goal for large arrays.

---

## 7. Bring-up order & what each result decides

1. `cargo build -p labview-test-library` (or let the just target do it).
2. Create `Variant Test Case.ctl`, `Bytes To Hex.vi`, `Cases Scalars.vi`,
   `Check Cases.vi`; run interactively. Scalars green ⇒ calling convention,
   descriptor parsing, and oracle plumbing all work.
3. Add categories in order: Strings → Arrays → Clusters → Nested → Special.
   Decision table for the designed-to-discriminate cases:

| Case | If it passes | If it fails |
|------|--------------|-------------|
| `time_cluster` | h5labview align-4 rule is right ⇒ `labview_layout!`/`repr(C)` mirrors with `LVTime` in clusters are wrong — flag crate-wide | garbage timestamp/tag ⇒ engine wrong ⇒ set Timestamp alignment to 8 in `layout.rs`, update `layout_crosscheck.rs` documented-discrepancy test to plain equality |
| `arr_of_clus` (Tail) | tail-padding stride (16) correct | elements 2/3 corrupt ⇒ stride bug in `read_array` / cluster `size()` |
| `pad_b` | every alignment class placed correctly | which field is wrong tells you which alignment rule is off |
| `enum_*` | enum headers parse as ported | 542007 ⇒ version-dependent enum padding gap — capture hex, fix parser |
| `empty_variant` | — | record actual behaviour, replace `*` with it |

4. Create the class + test methods, register in `rust-interop-test.lvproj`,
   run `just lv_ver=<ver> integration-tests-x64` — results land in `x64.xml`.
5. Run `Capture Golden Vectors.vi`, commit
   `labview-interop/tests/golden/captured.tsv`, confirm
   `cargo test -p labview-interop --test golden_typedesc` is green.
6. Later: `just integration-tests-x86` exercises the packed 32-bit layout
   with the *same* VIs and expected strings (canonical output is
   platform-independent — only the memory walking underneath differs).
