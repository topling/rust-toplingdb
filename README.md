# rust-toplingdb

[![ToplingDB build](https://github.com/topling/rust-toplingdb/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/topling/rust-toplingdb/actions/workflows/rust.yml)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/topling/rust-toplingdb/blob/master/LICENSE)
![rust 1.85 required](https://img.shields.io/badge/rust-1.85-blue.svg?label=MSRV)

ToplingDB Rust binding. Forked from
[rust-rocksdb](https://github.com/rust-rocksdb/rust-rocksdb).

For better compatibility with RocksDB, ToplingDB does not change the name
of the RocksDB shared library or the way existing RocksDB users interact
with it.

## Contributing

Feedback and pull requests welcome!

## Requirements

- System libraries: `libcurl4-openssl-dev`, `liburing-dev`, `libaio-dev`, `pkg-config`
- Compression libs (optional, auto-detected): `zlib1g-dev`, `libbz2-dev`, `liblz4-dev`, `libsnappy-dev`, `libzstd-dev`

## Usage

This binding dynamically links with `librocksdb.so` (ToplingDB keeps this
name unchanged to smooth migration of existing code). If you want to build
it yourself, make sure you've also cloned the submodules:

```shell
git submodule update --init --recursive
sudo yum install libcurl-devel # ToplingDB requires libcurl-devel
sudo apt install libcurl4-openssl-dev # for ubuntu & debian
cargo build
cargo test db::test_side_plugin_repo # ToplingDB side_plugin_repo
```

**Note**: After updating the repo, set `UPDATE_REPO=1` to rebuild
librocksdb: `UPDATE_REPO=1 cargo build`


## Jemalloc

`librocksdb.so` is built **without** jemalloc (`DISABLE_JEMALLOC=1`).
Its `malloc`/`free` calls resolve at runtime through ELF symbol
interposition.

This design avoids the crash that occurs when two jemalloc instances
coexist in the same process (e.g., if the outer Rust application
statically links jemalloc via `tikv-jemalloc-sys`). There is no jemalloc
Cargo feature on this crate.

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

The `tikv-jemalloc-sys` `unprefixed_malloc_on_supported_platforms` feature
is recommended so that jemalloc's `malloc`/`free` interpose libc's and are
visible to `librocksdb.so` at runtime.

## Cargo Features

### Default features

Default features are empty. No jemalloc, no compression features are
enabled by default — you must opt in to each explicitly.

### Compression support

Support for [Snappy](https://github.com/google/snappy),
[LZ4](https://github.com/lz4/lz4), [Zstd](https://github.com/facebook/zstd),
[Zlib](https://zlib.net), and [Bzip2](http://www.bzip.org) compression
is auto-detected when the corresponding development libraries are
installed on the system. Install the packages and rebuild
`librocksdb.so`:

```shell
sudo apt install zlib1g-dev libbz2-dev liblz4-dev libsnappy-dev libzstd-dev
cargo clean
cargo build
```

### Top-level features

| Feature | Description |
|---|---|
| `rtti` | Enable RTTI in librocksdb (propagated to `librocksdb-sys/rtti`) |
| `multi-threaded-cf` | Allow column family create/drop from multiple threads concurrently |
| `serde1` | Implement `Serialize`/`Deserialize` for public types |
| `valgrind` | Enable Valgrind-friendly options |

### librocksdb-sys features

| Feature | Description |
|---|---|
| `static` | **Deprecated.** Has no effect, linking is always dynamic |
| `mt_static` | On Windows, use `/MT` (static CRT) instead of `/MD` (dynamic CRT). No effect on Linux |
| `io-uring` | **Deprecated.** `io_uring` is always enabled in librocksdb.so |
| `rtti` | Enable RTTI when building librocksdb |

## Multithreaded ColumnFamily alternation

RocksDB allows column families to be created and dropped
from multiple threads concurrently, but this crate doesn't allow it by default
for compatibility. If you need to modify column families concurrently, enable
the crate feature `multi-threaded-cf`, which makes this binding's
data structures use `RwLock` by default. Alternatively, you can directly create
`DBWithThreadMode<MultiThreaded>` without enabling the crate feature.

