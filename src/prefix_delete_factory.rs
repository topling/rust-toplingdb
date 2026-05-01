//! ToplingDB side plugin: a CompactionFilterFactory that deletes keys whose
//! 4-byte prefix (big-endian) matches a configurable set.
//!
//! This module demonstrates the full side-plugin lifecycle:
//! - JSON-based configuration via `SidePluginRepo`
//! - `SerDe` / `WebManip` support for distributed compaction and web UI
//! - Thread-safe state sharing between factory and per-compaction filters
//!
//! # Registration
//!
//! The [`side_plugin!`](crate::side_plugin) macro registers the creator function
//! so that the C side can instantiate `PrefixDeleteFactory` from JSON config:
//!
//! ```json
//! {"CompactionFilterFactory": {"my_factory": {
//!     "class": "PrefixDeleteFactory",
//!     "params": {"prefixes": [1684104548]}
//! }}}
//! ```
//!
//! Prefixes in the JSON are native-endian `u32` values.  The key-matching logic
//! converts them to big-endian before comparison.  For example, `1684104548` in
//! JSON (i.e. `0x64656164` as hex) matches the ASCII prefix `"dead"`.

use crate::compaction_filter::{CompactionFilter, Decision};
use crate::compaction_filter_factory::{CompactionFilterContext, CompactionFilterFactory};
use crate::side_plugin;
use crate::side_plugin_ex::{SerDe, WebManip};
use crate::SidePluginRepo;
use serde_json;
use std::collections::BTreeSet;
use std::ffi::CStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

/// A [`CompactionFilterFactory`] that deletes keys matching configurable 4-byte
/// prefixes.
///
/// Thread-safe: the prefix set is protected by `RwLock`; deleted-key counting
/// uses `Arc<AtomicU64>`.  Per-compaction [`PrefixDeleteFilter`] instances
/// receive a snapshot of the prefix set and share the same atomic counter.
///
/// Supports [`SerDe`] for distributed compaction and [`WebManip`] for web UI.
pub struct PrefixDeleteFactory {
    /// Prefixes to delete (native-endian u32, converted internally to big-endian
    /// for key matching).
    pub prefixes: RwLock<BTreeSet<u32>>,
    /// Total number of keys deleted across all compactions that used this factory.
    pub total_deleted: Arc<AtomicU64>,
    /// Serialization format for SerDe (distributed compaction):
    /// - `false` (default): binary encoding (compact)
    /// - `true`: JSON encoding (human-readable, debuggable)
    ///
    /// This is a YAML/JSON configuration metadata option.  The framework
    /// propagates the entire `params` object automatically, so this flag
    /// does NOT need to be serialized by the SerDe trait — it is available
    /// on both sides from the shared config.
    pub serialize_as_json: bool,
}

/// Per-compaction filter snapshotting the prefix set at creation time.
pub struct PrefixDeleteFilter {
    /// Sorted snapshot of delete prefixes (native-endian u32).
    prefixes: Vec<u32>,
    /// Local count of keys deleted in this compaction run.
    local_deleted: u64,
    /// Shared counter — incremented on drop to sync back to the factory.
    total_deleted: Arc<AtomicU64>,
}

impl Drop for PrefixDeleteFilter {
    fn drop(&mut self) {
        self.total_deleted
            .fetch_add(self.local_deleted, Ordering::Relaxed);
    }
}

impl CompactionFilter for PrefixDeleteFilter {
    fn filter(&mut self, _level: u32, key: &[u8], _value: &[u8]) -> Decision {
        if key.len() >= 4 {
            let prefix = u32::from_be_bytes([key[0], key[1], key[2], key[3]]);
            if self.prefixes.binary_search(&prefix).is_ok() {
                self.local_deleted += 1;
                return Decision::Remove;
            }
        }
        Decision::Keep
    }
    fn name(&self) -> &CStr {
        unsafe { CStr::from_bytes_with_nul_unchecked(b"PrefixDeleteFactory\0") }
    }
}

impl CompactionFilterFactory for PrefixDeleteFactory {
    type Filter = PrefixDeleteFilter;

    fn create(&mut self, _context: CompactionFilterContext) -> PrefixDeleteFilter {
        let prefixes: Vec<u32> = self.prefixes.read().unwrap().iter().copied().collect();
        PrefixDeleteFilter {
            prefixes,
            local_deleted: 0,
            total_deleted: self.total_deleted.clone(),
        }
    }
    fn name(&self) -> &CStr {
        unsafe { CStr::from_bytes_with_nul_unchecked(b"PrefixDeleteFactory\0") }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SerDe — distributed compaction serialization
// ═══════════════════════════════════════════════════════════════════════════════

impl SerDe for PrefixDeleteFactory {
    fn serialize_request(&self, fp: *mut libc::FILE) {
        if self.serialize_as_json {
            // JSON format (human-readable, for debuggability).
            // `serialize_as_json` is YAML metadata (see struct docs) and is
            // NOT serialized here — the framework propagates the params
            // automatically, so both sides share the same flag value.
            let prefixes: Vec<u32> =
                self.prefixes.read().unwrap().iter().copied().collect();
            let json = serde_json::json!({ "prefixes": prefixes });
            let json_str = json.to_string();
            unsafe {
                libc::fwrite(
                    json_str.as_ptr() as *const libc::c_void,
                    1,
                    json_str.len(),
                    fp,
                );
            }
        } else {
            // Binary format (compact, default).
            let prefixes = self.prefixes.read().unwrap();
            let count = prefixes.len() as u64;
            unsafe {
                libc::fwrite(
                    &count as *const _ as *const libc::c_void,
                    std::mem::size_of::<u64>(),
                    1,
                    fp,
                );
                for &p in prefixes.iter() {
                    let be_bytes = p.to_be_bytes();
                    libc::fwrite(&be_bytes as *const _ as *const libc::c_void, 4, 1, fp);
                }
            }
        }
    }
    fn deserialize_request(&mut self, fp: *mut libc::FILE) {
        if self.serialize_as_json {
            // Read all remaining data from the FILE and parse as JSON.
            // The FILE is binary-opened, so reading raw bytes is fine.
            let mut buf = Vec::new();
            unsafe {
                let mut chunk = [0u8; 4096];
                loop {
                    let n = libc::fread(
                        chunk.as_mut_ptr() as *mut libc::c_void,
                        1,
                        chunk.len(),
                        fp,
                    );
                    if n == 0 {
                        break; // EOF
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) {
                if let Some(prefixes) = v.get("prefixes").and_then(|v| v.as_array()) {
                    let set: BTreeSet<u32> = prefixes
                        .iter()
                        .filter_map(|v| v.as_u64().map(|u| u as u32))
                        .collect();
                    *self.prefixes.write().unwrap() = set;
                }
            }
        } else {
            // Binary format (compact, default).
            let mut count: u64 = 0;
            unsafe {
                libc::fread(
                    &mut count as *mut _ as *mut libc::c_void,
                    std::mem::size_of::<u64>(),
                    1,
                    fp,
                );
            }
            let mut prefixes = BTreeSet::new();
            for _ in 0..count {
                let mut be_bytes = [0u8; 4];
                unsafe {
                    libc::fread(&mut be_bytes as *mut _ as *mut libc::c_void, 4, 1, fp);
                }
                prefixes.insert(u32::from_be_bytes(be_bytes));
            }
            *self.prefixes.write().unwrap() = prefixes;
        }
    }
    fn serialize_response(&self, fp: *mut libc::FILE) {
        if self.serialize_as_json {
            let count = self.total_deleted.load(Ordering::Relaxed);
            let json = serde_json::json!({ "total_deleted": count });
            let json_str = json.to_string();
            unsafe {
                libc::fwrite(
                    json_str.as_ptr() as *const libc::c_void,
                    1,
                    json_str.len(),
                    fp,
                );
            }
        } else {
            let count = self.total_deleted.load(Ordering::Relaxed);
            unsafe {
                libc::fwrite(
                    &count as *const _ as *const libc::c_void,
                    std::mem::size_of::<u64>(),
                    1,
                    fp,
                );
            }
        }
    }
    fn deserialize_response(&mut self, fp: *mut libc::FILE) {
        if self.serialize_as_json {
            let mut buf = Vec::new();
            unsafe {
                let mut chunk = [0u8; 4096];
                loop {
                    let n = libc::fread(
                        chunk.as_mut_ptr() as *mut libc::c_void,
                        1,
                        chunk.len(),
                        fp,
                    );
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) {
                if let Some(count) = v.get("total_deleted").and_then(|v| v.as_u64()) {
                    self.total_deleted.store(count, Ordering::Relaxed);
                }
            }
        } else {
            let mut count: u64 = 0;
            unsafe {
                libc::fread(
                    &mut count as *mut _ as *mut libc::c_void,
                    std::mem::size_of::<u64>(),
                    1,
                    fp,
                );
            }
            self.total_deleted.store(count, Ordering::Relaxed);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// WebManip — web UI bridge
// ═══════════════════════════════════════════════════════════════════════════════

impl WebManip for PrefixDeleteFactory {
    fn view(&self, _dump_options_json: &str, _repo: &SidePluginRepo) -> Vec<u8> {
        let deleted = self.total_deleted.load(Ordering::Relaxed);
        let prefixes: Vec<u32> = self.prefixes.read().unwrap().iter().copied().collect();
        let info = serde_json::json!({
            "total_deleted": deleted,
            "prefixes": prefixes,
            "serialize_as_json": self.serialize_as_json,
        });
        info.to_string().into_bytes()
    }
    fn update(
        &mut self,
        _dump_options_json: &str,
        body_json: &str,
        _repo: &SidePluginRepo,
    ) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(body_json) {
            if let Some(prefixes) = v.get("prefixes").and_then(|v| v.as_array()) {
                let set: BTreeSet<u32> = prefixes
                    .iter()
                    .filter_map(|v| v.as_u64().map(|u| u as u32))
                    .collect();
                *self.prefixes.write().unwrap() = set;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Creator function + side_plugin! registration
// ═══════════════════════════════════════════════════════════════════════════════

/// Creator function invoked by the C side when the JSON config references
/// `"class": "PrefixDeleteFactory"`.
///
/// The `json` parameter is the `params` object from the config:
///
/// ```json
/// {"prefixes": [1, 2, 3]}
/// ```
pub fn create(json: serde_json::Value, _repo: &SidePluginRepo) -> PrefixDeleteFactory {
    let prefixes = json
        .get("prefixes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|u| u as u32))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    // `serialize_as_json` is a metadata option from the YAML config:
    // it controls the SerDe wire format (JSON vs binary) but is NOT
    // serialized by the SerDe methods because the framework automatically
    // propagates the params JSON to all nodes in distributed compaction.
    let serialize_as_json = json
        .get("serialize_as_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let prefixes = RwLock::new(prefixes);

    PrefixDeleteFactory {
        prefixes,
        total_deleted: Arc::new(AtomicU64::new(0)),
        serialize_as_json,
    }
}

side_plugin!(compaction_filter_factory, "PrefixDeleteFactory", create);

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::side_plugin_ex::create_cff_c_object;
    use serde_json::json;

    // ── Unit: basic factory roundtrip ────────────────────────

    #[test]
    fn test_factory_create() {
        let json = json!({"prefixes": [0xdeadbeefu32, 0xcafebabeu32]});
        let factory = create(json, &SidePluginRepo::new());
        assert_eq!(
            *factory.prefixes.read().unwrap(),
            BTreeSet::from([0xdeadbeef, 0xcafebabe])
        );
        assert_eq!(factory.total_deleted.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_factory_empty_prefixes() {
        let json = json!({});
        let factory = create(json, &SidePluginRepo::new());
        assert!(factory.prefixes.read().unwrap().is_empty());
    }

    // ── Unit: CompactionFilter logic ─────────────────────────

    #[test]
    fn test_filter_remove_matching_prefix() {
        let mut factory = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::from([0x00000001, 0x00000002])),
            total_deleted: Arc::new(AtomicU64::new(0)),
            serialize_as_json: false,
        };
        let mut filter = factory.create(CompactionFilterContext {
            is_full_compaction: true,
            is_manual_compaction: false,
        });

        // Prefix 0x00000001 matches
        assert!(matches!(
            filter.filter(0, &[0x00, 0x00, 0x00, 0x01, 0xAA, 0xBB], b"val"),
            Decision::Remove,
        ));
        // Prefix 0x00000004 does not match
        assert!(matches!(
            filter.filter(0, &[0x00, 0x00, 0x00, 0x04, 0xCC, 0xDD], b"val"),
            Decision::Keep,
        ));
        // Short key
        assert!(matches!(filter.filter(0, b"ab", b"val"), Decision::Keep));

        assert_eq!(filter.local_deleted, 1);
        drop(filter);
        assert_eq!(factory.total_deleted.load(Ordering::Relaxed), 1);
    }

    // ── Unit: C-object roundtrip ─────────────────────────────

    #[test]
    fn test_c_object_roundtrip() {
        let factory = create(json!({"prefixes": [1, 2, 3]}), &SidePluginRepo::new());
        let ptr = create_cff_c_object(factory);
        assert!(!ptr.is_null());
    }

    // ── SerDe roundtrip ──────────────────────────────────────

    #[test]
    fn test_serde_request_roundtrip() {
        let factory = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::from([0x00000001, 0x00000002, 0x00000003])),
            total_deleted: Arc::new(AtomicU64::new(0)),
            serialize_as_json: false,
        };

        let fp = unsafe { libc::tmpfile() };
        assert!(!fp.is_null());
        factory.serialize_request(fp);
        unsafe { libc::rewind(fp) };

        let mut deserialized = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::new()),
            total_deleted: Arc::new(AtomicU64::new(0)),
            serialize_as_json: false,
        };
        deserialized.deserialize_request(fp);
        unsafe { libc::fclose(fp) };

        assert_eq!(
            *deserialized.prefixes.read().unwrap(),
            BTreeSet::from([0x00000001, 0x00000002, 0x00000003]),
        );
    }

    #[test]
    fn test_serde_response_roundtrip() {
        let factory = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::new()),
            total_deleted: Arc::new(AtomicU64::new(5)),
            serialize_as_json: false,
        };

        let fp = unsafe { libc::tmpfile() };
        assert!(!fp.is_null());
        factory.serialize_response(fp);
        unsafe { libc::rewind(fp) };

        let mut other = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::new()),
            total_deleted: Arc::new(AtomicU64::new(0)),
            serialize_as_json: false,
        };
        other.deserialize_response(fp);
        unsafe { libc::fclose(fp) };
        assert_eq!(other.total_deleted.load(Ordering::Relaxed), 5);
    }

    /// SerDe request roundtrip with JSON format (`serialize_as_json: true`).
    /// Verifies that the JSON branch reads back what the JSON branch wrote.
    #[test]
    fn test_serde_request_json_roundtrip() {
        let factory = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::from([0x00000001, 0x00000002, 0x00000003])),
            total_deleted: Arc::new(AtomicU64::new(0)),
            serialize_as_json: true,
        };

        let fp = unsafe { libc::tmpfile() };
        assert!(!fp.is_null());
        factory.serialize_request(fp);
        unsafe { libc::rewind(fp) };

        let mut deserialized = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::new()),
            total_deleted: Arc::new(AtomicU64::new(0)),
            serialize_as_json: true,
        };
        deserialized.deserialize_request(fp);
        unsafe { libc::fclose(fp) };

        assert_eq!(
            *deserialized.prefixes.read().unwrap(),
            BTreeSet::from([0x00000001, 0x00000002, 0x00000003]),
        );
    }

    // ── WebManip ─────────────────────────────────────────────

    #[test]
    fn test_webmanip_view_update() {
        let mut factory = PrefixDeleteFactory {
            prefixes: RwLock::new(BTreeSet::from([10, 20])),
            total_deleted: Arc::new(AtomicU64::new(3)),
            serialize_as_json: false,
        };

        // view
        let result = factory.view(r#"{}"#, &SidePluginRepo::new());
        let view_json: serde_json::Value =
            serde_json::from_str(&String::from_utf8(result).unwrap()).unwrap();
        assert_eq!(view_json["total_deleted"], json!(3));

        // update
        factory.update(r#"{}"#, r#"{"prefixes": [5, 6, 7]}"#, &SidePluginRepo::new());
        assert_eq!(*factory.prefixes.read().unwrap(), BTreeSet::from([5, 6, 7]));
    }

    // ── Integration: full DB workflow with SidePluginRepo ────

    /// Register via the C API directly (instead of relying on ctor) to
    /// ensure the plugin is available.  Then import the JSON config and
    /// exercise the full DB workflow.
    #[test]
    fn test_integration_db_compact_delete() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        // Manually register via C API (ctor-based registration does not
        // work because the C++ name map may not persist across translation
        // unit boundaries for inventory-collected entries).
        unsafe extern "C" fn local_thunk(
            strjson: *const libc::c_char,
            repo_ptr: *const crate::ffi::side_plugin_repo_t,
        ) -> *mut crate::ffi::rocksdb_compactionfilterfactory_t {
            let json_str = unsafe { std::ffi::CStr::from_ptr(strjson) }
                .to_str()
                .expect("invalid UTF-8");
            let json: serde_json::Value =
                serde_json::from_str(json_str).expect("invalid JSON");
            let repo = std::mem::ManuallyDrop::new(crate::SidePluginRepo {
                inner: repo_ptr as *mut _,
            });
            let result = create(json, &repo);
            create_cff_c_object(result)
        }
        let reg_name = std::ffi::CString::new("PrefixDeleteFactory").unwrap();
        unsafe {
            crate::ffi::side_plugin_register_compaction_filter_factory(
                reg_name.as_ptr(),
                Some(local_thunk),
                std::ptr::null(),
            );
        }

        let repo = SidePluginRepo::new();
        let config = serde_json::json!({
            "CompactionFilterFactory": {
                "my_factory": {
                    "class": "PrefixDeleteFactory",
                    "params": {
                        // "dead" as big-endian u32 = 0x64656164
                        "prefixes": [0x64656164u32]
                    }
                }
            },
            "CFOptions": {
                "default": {
                    "compaction_filter_factory": "$my_factory",
                    "disable_auto_compactions": true
                }
            },
            "DBOptions": {
                "dbo": {
                    "create_if_missing": true,
                    "create_missing_column_families": true
                }
            },
            "databases": {
                "test_db": {
                    "method": "DB::Open",
                    "params": {
                        "db_options": "$dbo",
                        "cf_options": "$default",
                        "path": path
                    }
                }
            },
            "open": "test_db"
        });

        repo.import(&config.to_string()).unwrap();
        let db = repo.open().unwrap();

        // Insert keys — ones prefixed "dead" match the configured filter,
        // ones prefixed "live" do not.
        db.put(b"dead:001", b"delete_me").unwrap();
        db.put(b"dead:002", b"delete_me_too").unwrap();
        db.put(b"live:001", b"keep_me").unwrap();
        db.put(b"live:002", b"keep_me_too").unwrap();

        // Verify the to-be-deleted keys exist before compaction.
        assert_eq!(
            db.get(b"dead:001").unwrap().as_deref(),
            Some(&b"delete_me"[..]),
        );
        assert_eq!(
            db.get(b"dead:002").unwrap().as_deref(),
            Some(&b"delete_me_too"[..]),
        );

        // Flush to SST then compact to trigger the compaction filter.
        db.flush().unwrap();
        db.compact_range::<&[u8], &[u8]>(None, None);

        // Verify: "dead"-prefixed keys removed, "live"-prefixed kept.
        assert_eq!(db.get(b"dead:001").unwrap(), None);
        assert_eq!(db.get(b"dead:002").unwrap(), None);
        assert_eq!(
            db.get(b"live:001").unwrap().as_deref(),
            Some(&b"keep_me"[..]),
        );
        assert_eq!(
            db.get(b"live:002").unwrap().as_deref(),
            Some(&b"keep_me_too"[..]),
        );

        dir.close().unwrap();
    }

}
