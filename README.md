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


## Jemalloc

`librocksdb.so` is built **without** jemalloc (`DISABLE_JEMALLOC=1`). Its
`malloc`/`free` calls resolve at runtime through ELF symbol interposition.

This design avoids the crash that occurs when two jemalloc instances coexist in
the same process (e.g., if the outer Rust application statically links jemalloc
via `tikv-jemalloc-sys`). There is no jemalloc Cargo feature on this crate.

### Using jemalloc in your application

To use jemalloc as the global allocator, add it to your own `Cargo.toml`:

```toml
[dependencies]
tikv-jemallocator = "0.5"
```

Then in your `src/main.rs`:

```rust
use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;
```

The `tikv-jemalloc-sys` `unprefixed_malloc_on_supported_platforms` feature is
recommended so that jemalloc's `malloc`/`free` interpose libc's and are visible
to `librocksdb.so` at runtime.

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
