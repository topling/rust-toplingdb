use crate::ffi;
use crate::SidePluginRepo;
use libc;
use std::ffi::CStr;
use std::os::raw::c_void;

/// Trait for distributed compaction serialization/deserialization (4 phases).
///
/// Implement this on your plugin type to support ToplingDB distributed compaction.
/// The `state_` pointer registered via C creation functions must point to your type.
pub trait SerDe {
    fn serialize_request(&self, fp: *mut libc::FILE);
    fn deserialize_request(&mut self, fp: *mut libc::FILE);
    fn serialize_response(&self, fp: *mut libc::FILE);
    fn deserialize_response(&mut self, fp: *mut libc::FILE);
}

/// Trait for web view/update bridge.
///
/// Implement this on your plugin type to support ToplingDB web UI manipulation.
/// The `state_` pointer registered via C creation functions must point to your type.
///
/// The `dump_options_json` parameter typically contains two fields:
/// - `html` (bool): whether the return value is HTML or JSON, defaults to `true`
/// - `verbose` (int): detail level (higher = more verbose), defaults to `0`
pub trait WebManip {
    fn view(&self, dump_options_json: &str, repo: &SidePluginRepo) -> Vec<u8>;
    fn update(&mut self, dump_options_json: &str, body_json: &str, repo: &SidePluginRepo);
}

// ── Conversion helpers (thunk uses these to turn existing safe Rust types into C pointers) ──

/// Convert a safe `ComparatorCallback` into a C `rocksdb_comparator_t` pointer.
pub fn create_comparator_c_object(
    cb: crate::comparator::ComparatorCallback,
) -> *const ffi::rocksdb_comparator_t {
    let ptr = Box::into_raw(Box::new(cb));
    unsafe {
        ffi::rocksdb_comparator_create(
            ptr as *mut c_void,
            Some(crate::comparator::ComparatorCallback::destructor_callback),
            Some(crate::comparator::ComparatorCallback::compare_callback),
            Some(crate::comparator::ComparatorCallback::name_callback),
        )
    }
}

/// Convert a safe `MergeOperatorCallback` into a C `rocksdb_mergeoperator_t` pointer.
pub fn create_merge_op_c_object<F, PF>(
    cb: crate::merge_operator::MergeOperatorCallback<F, PF>,
) -> *mut ffi::rocksdb_mergeoperator_t
where
    F: crate::merge_operator::MergeFn,
    PF: crate::merge_operator::MergeFn,
{
    let ptr = Box::into_raw(Box::new(cb));
    unsafe {
        ffi::rocksdb_mergeoperator_create(
            ptr as *mut c_void,
            Some(crate::merge_operator::destructor_callback::<F, PF>),
            Some(crate::merge_operator::full_merge_callback::<F, PF>),
            Some(crate::merge_operator::partial_merge_callback::<F, PF>),
            Some(crate::merge_operator::delete_callback),
            Some(crate::merge_operator::name_callback::<F, PF>),
        )
    }
}

/// Convert a safe `CompactionFilterFactory` impl into a C `rocksdb_compactionfilterfactory_t` pointer.
pub fn create_cff_c_object<F>(factory: F) -> *mut ffi::rocksdb_compactionfilterfactory_t
where
    F: crate::compaction_filter_factory::CompactionFilterFactory + 'static,
{
    let ptr = Box::into_raw(Box::new(factory));
    unsafe {
        ffi::rocksdb_compactionfilterfactory_create(
            ptr as *mut c_void,
            Some(crate::compaction_filter_factory::destructor_callback::<F>),
            Some(crate::compaction_filter_factory::create_compaction_filter_callback::<F>),
            Some(crate::compaction_filter_factory::name_callback::<F>),
        )
    }
}

/// Convert a safe `SliceTransform` into a C `rocksdb_slicetransform_t` pointer.
pub fn create_slice_transform_c_object(
    st: crate::SliceTransform,
) -> *mut ffi::rocksdb_slicetransform_t {
    st.inner
}

// ── Public helpers for constructing existing safe types ──────────────────

/// Construct a `ComparatorCallback` from its parts.
/// `ComparatorCallback` is in a private module; this helper is the public constructor.
pub fn make_comparator_callback(
    name: std::ffi::CString,
    compare_fn: Box<dyn Fn(&[u8], &[u8]) -> std::cmp::Ordering + Send + Sync>,
) -> crate::comparator::ComparatorCallback {
    crate::comparator::ComparatorCallback { name, compare_fn }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Inventory entry types (collected at link time)
// ═══════════════════════════════════════════════════════════════════════════════

/// Comparator plugin entry — submitted by `side_plugin!(comparator, ...)`.
#[doc(hidden)]
pub struct CompPluginEntry {
    pub name: &'static CStr,
    pub register: fn(&'static CStr),
}
inventory::collect!(CompPluginEntry);

/// MergeOperator plugin entry — submitted by `side_plugin!(merge_operator, ...)`.
#[doc(hidden)]
pub struct MergePluginEntry {
    pub name: &'static CStr,
    pub register: fn(&'static CStr),
}
inventory::collect!(MergePluginEntry);

/// CompactionFilterFactory plugin entry — submitted by `side_plugin!(compaction_filter_factory, ...)`.
#[doc(hidden)]
pub struct CompFilterFactoryPluginEntry {
    pub name: &'static CStr,
    pub register: fn(&'static CStr),
}
inventory::collect!(CompFilterFactoryPluginEntry);

/// SliceTransform plugin entry — submitted by `side_plugin!(slice_transform, ...)`.
#[doc(hidden)]
pub struct SliceTransformPluginEntry {
    pub name: &'static CStr,
    pub register: fn(&'static CStr),
}
inventory::collect!(SliceTransformPluginEntry);

// ═══════════════════════════════════════════════════════════════════════════════
// Auto-registration at startup (before main)
// ═══════════════════════════════════════════════════════════════════════════════

/// Called before `main()` via `ctor` — iterates all inventory collections
/// and calls the register function for every submitted plugin entry.
#[doc(hidden)]
#[ctor::ctor]
unsafe fn __auto_register_all_plugins() {
    for entry in inventory::iter::<CompPluginEntry> {
        (entry.register)(entry.name);
    }
    for entry in inventory::iter::<MergePluginEntry> {
        (entry.register)(entry.name);
    }
    for entry in inventory::iter::<CompFilterFactoryPluginEntry> {
        (entry.register)(entry.name);
    }
    for entry in inventory::iter::<SliceTransformPluginEntry> {
        (entry.register)(entry.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction_filter::{CompactionFilter, Decision};
    use crate::compaction_filter_factory::{CompactionFilterContext, CompactionFilterFactory};
    use crate::comparator::ComparatorCallback;
    use crate::merge_operator::{MergeFn, MergeOperatorCallback};
    use crate::side_plugin;
    use crate::SliceTransform;
    use serde_json::json;
    use std::ffi::CString;

    // ── Comparator ─────────────────────────────────────────────────────────────────
    //
    // User writes a plain function, no trait to impl:

    fn test_comp_creator(_json: serde_json::Value, _repo: &SidePluginRepo) -> ComparatorCallback {
        make_comparator_callback(
            CString::new("test-comp").unwrap(),
            Box::new(|a: &[u8], b: &[u8]| a.cmp(b)),
        )
    }

    #[test]
    fn test_comparator_roundtrip() {
        let repo = SidePluginRepo::new();
        let cb = test_comp_creator(json!({"name": "test-comp"}), &repo);
        let ptr = create_comparator_c_object(cb);
        assert!(!ptr.is_null());
    }

    // ── MergeOperator ──────────────────────────────────────────────────────────────

    fn test_merge_creator(
        _json: serde_json::Value,
        _repo: &SidePluginRepo,
    ) -> MergeOperatorCallback<impl MergeFn, impl MergeFn> {
        MergeOperatorCallback {
            name: CString::new("test-merge").unwrap(),
            full_merge_fn: |_key, _old, _ops| None,
            partial_merge_fn: |_key, _old, _ops| None,
        }
    }

    #[test]
    fn test_merge_roundtrip() {
        let repo = SidePluginRepo::new();
        let cb = test_merge_creator(json!({}), &repo);
        let ptr = create_merge_op_c_object(cb);
        assert!(!ptr.is_null());
    }

    // ── CompactionFilterFactory ────────────────────────────────────────────────────

    struct KeepFilter;
    impl CompactionFilter for KeepFilter {
        fn filter(&mut self, _level: u32, _key: &[u8], _value: &[u8]) -> Decision {
            Decision::Keep
        }
        fn name(&self) -> &std::ffi::CStr {
            unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(b"KeepFilter\0") }
        }
    }

    struct KeepFactory;
    impl CompactionFilterFactory for KeepFactory {
        type Filter = KeepFilter;
        fn create(&mut self, _ctx: CompactionFilterContext) -> KeepFilter {
            KeepFilter
        }
        fn name(&self) -> &std::ffi::CStr {
            unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(b"KeepFactory\0") }
        }
    }

    fn test_factory_creator(
        _json: serde_json::Value,
        _repo: &SidePluginRepo,
    ) -> impl CompactionFilterFactory<Filter = KeepFilter> {
        KeepFactory
    }

    #[test]
    fn test_factory_roundtrip() {
        let repo = SidePluginRepo::new();
        let factory = test_factory_creator(json!({}), &repo);
        let ptr = create_cff_c_object(factory);
        assert!(!ptr.is_null());
    }

    // ── SliceTransform ─────────────────────────────────────────────────────────────

    fn test_xform_creator(json: serde_json::Value, _repo: &SidePluginRepo) -> SliceTransform {
        let len = json.get("len").and_then(|v| v.as_u64()).unwrap_or(4) as usize;
        SliceTransform::create_fixed_prefix(len)
    }

    #[test]
    fn test_xform_roundtrip() {
        let repo = SidePluginRepo::new();
        let st = test_xform_creator(json!({"len": 4}), &repo);
        let ptr = create_slice_transform_c_object(st);
        assert!(!ptr.is_null());
    }

    // ── side_plugin! macro compiles ────────────────────────────────────────────────
    //
    // These invocations demonstrate the intended registration API.
    // Registration (inventory + ctor) happens before main() – no manual call needed.

    side_plugin!(comparator, "test-macro-comp", test_comp_creator);
    side_plugin!(merge_operator, "test-macro-merge", test_merge_creator);
    side_plugin!(
        compaction_filter_factory,
        "test-macro-factory",
        test_factory_creator
    );
    side_plugin!(slice_transform, "test-macro-xform", test_xform_creator);
}
