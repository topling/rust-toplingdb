# rust-rocksdb

[![RocksDB build](https://github.com/rust-rocksdb/rust-rocksdb/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/rust-rocksdb/rust-rocksdb/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/rocksdb.svg)](https://crates.io/crates/rocksdb)
[![documentation](https://docs.rs/rocksdb/badge.svg)](https://docs.rs/rocksdb)
[![license](https://img.shields.io/crates/l/rocksdb.svg)](https://github.com/rust-rocksdb/rust-rocksdb/blob/master/LICENSE)
[![Gitter chat](https://badges.gitter.im/rust-rocksdb/gitter.svg)](https://gitter.im/rust-rocksdb/lobby)
![rust 1.70.0 required](https://img.shields.io/badge/rust-1.70.0-blue.svg?label=MSRV)

![GitHub commits (since latest release)](https://img.shields.io/github/commits-since/rust-rocksdb/rust-rocksdb/latest.svg)

## Requirements

- Clang and LLVM

## Contributing

Feedback and pull requests welcome! If a particular feature of RocksDB is
important to you, please let me know by opening an issue, and I'll
prioritize it.

## Usage

This binding is statically linked with a specific version of RocksDB. If you
want to build it yourself, make sure you've also cloned the RocksDB and
compression submodules:

```shell
git submodule update --init --recursive
sudo yum install libcurl-devel # ToplingDB requires libcurl-devel
sudo apt install libcurl4-openssl-dev # for ubuntu & debian
cargo build
cargo test db::test_side_plugin_repo # ToplingDB side_plugin_repo
```

**Note**: If the repo is updated later, use UPDATE_REPO=1 as above


## Jemalloc Control

ToplingDB uses jemalloc as the default memory allocator. The RocksDB C++ library
(`librocksdb.so`) is built with jemalloc, and the Rust side links jemalloc to
ensure allocator consistency.

### Disable jemalloc

The `jemalloc` Cargo feature is enabled by default. To disable it:

```shell
cargo build --no-default-features
```

Or in your `Cargo.toml`:

```toml
[dependencies.rocksdb]
default-features = false
```

**Note**: Switching jemalloc on/off changes how `librocksdb.so` is compiled. Run
`make -C librocksdb-sys/rocksdb clean` before rebuilding to ensure the C++
library picks up the new setting.

### Consistency check

If the `DISABLE_JEMALLOC` environment variable is set, build.rs verifies it
matches the Cargo feature to prevent inconsistency:

| `DISABLE_JEMALLOC` | feature `jemalloc` | Result |
|---|---|---|
| (unset) | enabled (default) | ✅ jemalloc enabled |
| (unset) | disabled | ✅ jemalloc disabled |
| `1` | disabled | ✅ jemalloc disabled |
| `0` | enabled | ✅ jemalloc enabled |
| `1` | enabled | ❌ compile error |
| `0` | disabled | ❌ compile error |

## Compression Support

By default, support for [Snappy](https://github.com/google/snappy),
[LZ4](https://github.com/lz4/lz4), [Zstd](https://github.com/facebook/zstd),
[Zlib](https://zlib.net), and [Bzip2](http://www.bzip.org) compression
is enabled through crate features. If support for all of these compression
algorithms is not needed, default features can be disabled and specific
compression algorithms can be enabled. For example, to enable only LZ4
compression support, make these changes to your Cargo.toml:

```toml
[dependencies.rocksdb]
default-features = false
features = ["lz4"]
```

## Multithreaded ColumnFamily alternation

RocksDB allows column families to be created and dropped
from multiple threads concurrently, but this crate doesn't allow it by default
for compatibility. If you need to modify column families concurrently, enable
the crate feature `multi-threaded-cf`, which makes this binding's
data structures use `RwLock` by default. Alternatively, you can directly create
`DBWithThreadMode<MultiThreaded>` without enabling the crate feature.

## Switch between /MT or /MD run time library (Only for Windows)

The feature `mt_static` will request the library to be built with [/MT](https://learn.microsoft.com/en-us/cpp/build/reference/md-mt-ld-use-run-time-library?view=msvc-170)
flag, which results in library using the static version of the run-time library.
*This can be useful in case there's a conflict in the dependecy tree between different
run-time versions.*
