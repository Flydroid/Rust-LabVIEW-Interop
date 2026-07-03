//! Cross-checks the `typedesc` layout engine against `labview_layout!`
//! mirror structs — the same struct layout the rest of the crate already
//! relies on for cluster interop.
//!
//! On Windows these two must agree: Win64 LabVIEW uses natural C alignment
//! capped at 8 (= `repr(C)`), and Win32 LabVIEW is packed
//! (= `repr(C, packed)`, which `labview_layout!` applies on 32-bit).
//!
//! On Linux/macOS the layout engine caps alignment at 4 (per h5labview)
//! while `repr(C)` aligns f64/u64 to 8, so the two intentionally disagree
//! and this cross-check is Windows-only. Ground truth for non-Windows
//! platforms must come from LabVIEW golden vectors.
//!
//! NOTE: agreement here does not prove the layout matches LabVIEW — both
//! sides could share a wrong assumption. The LabVIEW integration tests
//! (`variant_to_canonical_string` against the torture clusters) are the
//! ground truth; this test just catches layout-engine regressions cheaply.
#![cfg(target_os = "windows")]

use labview_interop::labview_layout;
use labview_interop::typedesc::{LvTypeCode, TypeDescriptor};
use labview_interop::types::{LStrHandle, LVArrayHandle, LVBool, LVTime};
use std::mem::{offset_of, size_of};

fn num(code: LvTypeCode) -> TypeDescriptor {
    TypeDescriptor::Numeric { code, name: None }
}

fn cluster(fields: Vec<TypeDescriptor>) -> TypeDescriptor {
    TypeDescriptor::Cluster {
        fields,
        name: None,
    }
}

fn array(ndims: u16, element: TypeDescriptor) -> TypeDescriptor {
    TypeDescriptor::Array {
        ndims,
        element: Box::new(element),
        name: None,
    }
}

labview_layout!(
    pub struct PadA {
        a: u8,
        b: u64,
    }
);

#[test]
fn pad_a_classic_padding() {
    let td = cluster(vec![num(LvTypeCode::U8), num(LvTypeCode::U64)]);
    assert_eq!(td.offset_of(0), Some(offset_of!(PadA, a)));
    assert_eq!(td.offset_of(1), Some(offset_of!(PadA, b)));
    assert_eq!(td.size(), size_of::<PadA>());
}

labview_layout!(
    pub struct PadB {
        a: u8,
        b: u16,
        c: u8,
        d: u32,
        e: u8,
        f: u64,
    }
);

#[test]
fn pad_b_every_alignment_class() {
    let td = cluster(vec![
        num(LvTypeCode::U8),
        num(LvTypeCode::U16),
        num(LvTypeCode::U8),
        num(LvTypeCode::U32),
        num(LvTypeCode::U8),
        num(LvTypeCode::U64),
    ]);
    assert_eq!(td.offset_of(0), Some(offset_of!(PadB, a)));
    assert_eq!(td.offset_of(1), Some(offset_of!(PadB, b)));
    assert_eq!(td.offset_of(2), Some(offset_of!(PadB, c)));
    assert_eq!(td.offset_of(3), Some(offset_of!(PadB, d)));
    assert_eq!(td.offset_of(4), Some(offset_of!(PadB, e)));
    assert_eq!(td.offset_of(5), Some(offset_of!(PadB, f)));
    assert_eq!(td.size(), size_of::<PadB>());
}

labview_layout!(
    pub struct Tail {
        a: u64,
        b: u8,
    }
);

#[test]
fn tail_padding_gives_array_stride() {
    // The cluster's size (including tail padding) is the stride of an
    // array-of-clusters — the array-of-cluster killer case.
    let td = cluster(vec![num(LvTypeCode::U64), num(LvTypeCode::U8)]);
    assert_eq!(td.size(), size_of::<Tail>());
    #[cfg(target_pointer_width = "64")]
    assert_eq!(td.size(), 16); // NOT 9
}

labview_layout!(
    pub struct NestedInner {
        x: u8,
        y: u32,
    }
);

labview_layout!(
    pub struct NestedMid {
        a: u8,
        inner: NestedInner,
        b: u8,
    }
);

#[test]
fn nested_cluster_alignment() {
    let inner = cluster(vec![num(LvTypeCode::U8), num(LvTypeCode::U32)]);
    let td = cluster(vec![num(LvTypeCode::U8), inner, num(LvTypeCode::U8)]);
    assert_eq!(td.offset_of(0), Some(offset_of!(NestedMid, a)));
    assert_eq!(td.offset_of(1), Some(offset_of!(NestedMid, inner)));
    assert_eq!(td.offset_of(2), Some(offset_of!(NestedMid, b)));
    assert_eq!(td.size(), size_of::<NestedMid>());
}

labview_layout!(
    pub struct DeepC {
        v: i32,
    }
);
labview_layout!(
    pub struct DeepB {
        c: DeepC,
    }
);
labview_layout!(
    pub struct DeepA {
        b: DeepB,
    }
);

#[test]
fn deeply_nested_clusters() {
    let td = cluster(vec![cluster(vec![cluster(vec![num(LvTypeCode::I32)])])]);
    assert_eq!(td.size(), size_of::<DeepA>());
    assert_eq!(td.offset_of(0), Some(offset_of!(DeepA, b)));
}

labview_layout!(
    pub struct HandleMix<'a> {
        f1: LVBool,
        name: LStrHandle<'a>,
        f2: LVBool,
        data: LVArrayHandle<'a, 1, f64>,
        f3: LVBool,
    }
);

#[test]
fn handles_interleaved_with_bools() {
    let td = cluster(vec![
        TypeDescriptor::Boolean { name: None },
        TypeDescriptor::String { name: None },
        TypeDescriptor::Boolean { name: None },
        array(1, num(LvTypeCode::Dbl)),
        TypeDescriptor::Boolean { name: None },
    ]);
    assert_eq!(td.offset_of(0), Some(offset_of!(HandleMix<'static>, f1)));
    assert_eq!(td.offset_of(1), Some(offset_of!(HandleMix<'static>, name)));
    assert_eq!(td.offset_of(2), Some(offset_of!(HandleMix<'static>, f2)));
    assert_eq!(td.offset_of(3), Some(offset_of!(HandleMix<'static>, data)));
    assert_eq!(td.offset_of(4), Some(offset_of!(HandleMix<'static>, f3)));
    assert_eq!(td.size(), size_of::<HandleMix<'static>>());
}

labview_layout!(
    pub struct ArrInClus<'a> {
        id: u16,
        values: LVArrayHandle<'a, 1, i32>,
        mat: LVArrayHandle<'a, 2, f64>,
    }
);

#[test]
fn arrays_inside_cluster() {
    let td = cluster(vec![
        num(LvTypeCode::U16),
        array(1, num(LvTypeCode::I32)),
        array(2, num(LvTypeCode::Dbl)),
    ]);
    assert_eq!(td.offset_of(0), Some(offset_of!(ArrInClus<'static>, id)));
    assert_eq!(
        td.offset_of(1),
        Some(offset_of!(ArrInClus<'static>, values))
    );
    assert_eq!(td.offset_of(2), Some(offset_of!(ArrInClus<'static>, mat)));
    assert_eq!(td.size(), size_of::<ArrInClus<'static>>());
}

labview_layout!(
    pub struct TimeCluster {
        tag: u8,
        t: LVTime,
    }
);

/// KNOWN OPEN DISCREPANCY — do not "fix" either side until the LabVIEW
/// integration test decides.
///
/// The layout engine follows h5labview and aligns timestamps to 4; a
/// `repr(C)` struct with `LVTime` (i64 + u64) aligns to 8. On Win64 that
/// means `{u8, timestamp}` puts the timestamp at offset 4 (engine) vs
/// offset 8 (repr(C)). The `TimeCluster` torture case in the LabVIEW test
/// VIs is designed to resolve this; whichever side is wrong must then be
/// corrected and this test updated to a plain equality check.
#[test]
fn timestamp_alignment_discrepancy_documented() {
    let td = cluster(vec![
        num(LvTypeCode::U8),
        TypeDescriptor::Timestamp { name: None },
    ]);
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(td.offset_of(1), Some(4), "layout engine (h5labview rule)");
        assert_eq!(offset_of!(TimeCluster, t), 8, "repr(C) natural alignment");
    }
    #[cfg(target_pointer_width = "32")]
    {
        // Packed: both agree.
        assert_eq!(td.offset_of(1), Some(offset_of!(TimeCluster, t)));
        assert_eq!(td.size(), size_of::<TimeCluster>());
    }
}
