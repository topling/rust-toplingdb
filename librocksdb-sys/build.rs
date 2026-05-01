use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Generate Rust FFI bindings from rocksdb C header
    let rocksdb_include = env::var("ROCKSDB_INCLUDE_DIR").unwrap_or_else(|_| "rocksdb/include".into());
    let bindings = bindgen::Builder::default()
        .header(format!("{rocksdb_include}/rocksdb/c.h"))
        .derive_debug(false)
        .blocklist_type("max_align_t") // https://github.com/rust-lang-nursery/rust-bindgen/issues/550
        .ctypes_prefix("libc")
        .size_t_is_usize(true)
        .generate()
        .expect("unable to generate rocksdb bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("unable to write rocksdb bindings");

    // Determine whether jemalloc is enabled:
    //   - DISABLE_JEMALLOC env var, if explicitly set, takes precedence (backward compat)
    //   - Otherwise, derive from Cargo feature jemalloc (CARGO_FEATURE_JEMALLOC=1 when enabled)
    let user_disable = env::var("DISABLE_JEMALLOC");
    let feature_enabled = env::var("CARGO_FEATURE_JEMALLOC").is_ok();
    if let Ok(v) = &user_disable {
        if (*v == "1") == feature_enabled {
            panic!(
                "DISABLE_JEMALLOC={} is inconsistent with cargo feature jemalloc={}:\n  \
                 If jemalloc feature is enabled, set DISABLE_JEMALLOC=0;\n  \
                 If jemalloc feature is disabled, set DISABLE_JEMALLOC=1.\n  \
                 Or omit DISABLE_JEMALLOC entirely and let cargo derive it from the feature.",
                v,
                if feature_enabled { "enabled" } else { "disabled" }
            );
        }
    }
    let disable_jemalloc = match user_disable {
        Ok(v) => v == "1",
        Err(_) => !feature_enabled,
    };
    println!("cargo:rerun-if-env-changed=DISABLE_JEMALLOC");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_JEMALLOC");

    // Find or build librocksdb.so
    let rocksdb_lib_dir = if let Ok(dir) = env::var("ROCKSDB_LIB_DIR") {
        PathBuf::from(dir)
    } else {
        println!("cargo:rerun-if-changed=rocksdb/");
        let debug_level = match env::var("PROFILE").unwrap().as_str() {
            "release" => "0",
            _ => "2",
        };
        let num_jobs = env::var("CARGO_BUILD_JOBS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(num_cpus::get);
        let status = Command::new("make")
            .current_dir("rocksdb")
            .args(["shared_lib", "UPDATE_REPO=0", "LIB_MODE=shared", &format!("DEBUG_LEVEL={debug_level}"), "USE_LTO=1", &format!("-j{num_jobs}")])
            .env("DISABLE_JEMALLOC", if disable_jemalloc { "1" } else { "0" })
            .status()
            .expect("failed to run 'make shared_lib' for rocksdb");
        assert!(status.success(), "'make shared_lib' failed");
        env::var("CARGO_MANIFEST_DIR").map(|d| PathBuf::from(d).join("rocksdb")).unwrap()
    };

    // Copy librocksdb.so* to the cargo deps output directory (target/{profile}/deps/)
    // so the dynamic loader can find it at runtime. Cargo includes this directory
    // in LD_LIBRARY_PATH when running tests and binaries.
    let target_deps = out_path.ancestors().nth(3).unwrap().join("deps");
    for entry in fs::read_dir(&rocksdb_lib_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("librocksdb.so") && (entry.file_type().map(|t| t.is_file() || t.is_symlink()).unwrap_or(false)) {
            let dest = target_deps.join(&name);
            let _ = fs::remove_file(&dest);
            if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
                let link_target = fs::read_link(entry.path()).unwrap();
                std::os::unix::fs::symlink(link_target, &dest).unwrap();
            } else {
                fs::copy(entry.path(), &dest).unwrap();
            }
        }
    }

    println!("cargo:rustc-link-search=native={}", rocksdb_lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=rocksdb");

    // Link C++ runtime (librocksdb.so is a C++ library)
    let target = env::var("TARGET").unwrap();
    if target.contains("linux") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        if !disable_jemalloc {
            // librocksdb.so uses jemalloc internally; the executable must
            // explicitly link jemalloc so its malloc/free override libc's
            // and remain compatible with jemalloc-specific functions (mallocx, etc.)
            // Use --no-default-features or DISABLE_JEMALLOC=1 to disable.
            println!("cargo:rustc-link-lib=dylib=jemalloc");
        }
    } else if target.contains("apple") || target.contains("freebsd") || target.contains("openbsd") {
        println!("cargo:rustc-link-lib=dylib=c++");
    }

    // Metadata for downstream crates
    println!(
        "cargo:cargo_manifest_dir={}",
        env::var("CARGO_MANIFEST_DIR").unwrap()
    );
    println!("cargo:out_dir={}", out_path.display());
}
