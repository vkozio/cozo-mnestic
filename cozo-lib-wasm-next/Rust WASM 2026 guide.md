# Engineering Guide: High-Performance, Multi-Threaded Rust on WebAssembly

This document provides a production-grade blueprint for engineering high-throughput, multi-threaded WebAssembly (Wasm) applications in modern web browsers. It specifically addresses architectures with intensive computational and I/O demands, such as embedded database kernels, analytical query engines, and persistent storage layers.

---

## 1. Toolchain Matrix & Environment Setup

Building multi-threaded WebAssembly with hardware vectorization requires a pinned Rust **Nightly** toolchain. The `wasm32-unknown-unknown` target does not ship pre-compiled standard library binaries with thread support; the runtime standard library must be recompiled on the fly via `-Z build-std`.

### Required Tools

| Tool | Role | Minimum Tested Version |
| :--- | :--- | :--- |
| **Rust Toolchain** | Compiler & standard library source | `nightly-2024-10-01`+ |
| **`wasm-bindgen-cli`** | Generates JS/Wasm glue bindings | `0.2.95`+ |
| **`wasm-opt` (Binaryen)** | Post-link Wasm optimization pass | `version_118`+ |
| **Trunk** | Development server & asset bundler | `0.20.0`+ |
| **mise** (or Cargo) | Hermetic CLI version management | Latest |

### Environment Setup (`mise.toml`)

Create a `mise.toml` in your project root to eliminate host-environment drift across Linux, macOS, and Windows:

```toml
[tools]
rust = "nightly"
"ubi:rustwasm/wasm-bindgen-cli" = "latest"
"ubi:WebAssembly/binaryen" = "latest"
"cargo:trunk" = "latest"
```

Install components required for compiler-level rebuilds:

```bash
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
rustup target add wasm32-unknown-unknown --toolchain nightly
```

---

## 2. Low-Level Target, Linker, & TLS Configuration

Multi-threaded Wasm requires WebAssembly atomic operations, shared linear memory, and thread-local storage (TLS) exports.

### Linker Requirements for `lld`
When compiling with `target-feature=+atomics`, LLVM requires explicit linker instructions:
1. `--shared-memory` flags the Wasm memory definition as shared across agents.
2. `--import-memory` forces the Wasm instance to receive its memory from JavaScript (as a `SharedArrayBuffer`).
3. `--export=__wasm_init_tls` alongside symbols `__tls_size`, `__tls_align`, and `__tls_base` must be preserved in the binary. Without these flags, `lld` strips the initialization metadata required by `wasm-bindgen-rayon` when child workers boot.
4. Memory boundaries: WebAssembly 32-bit architecture enforces a hard ceiling of 4 GiB (`4294967296` bytes). However, modern browser engines (especially WebKit/Safari and mobile Chromium) frequently reject allocations exceeding **2 GiB** for a `SharedArrayBuffer`. Setting an explicit maximum of 2 GiB (`2147483648` bytes) guarantees cross-browser allocation stability.

### `.cargo/config.toml`

```toml
[target.wasm32-unknown-unknown]
rustflags = [
    # 1. Target CPU capabilities
    "-C", "target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128",

    # 2. Shared memory configuration
    "-C", "link-arg=--shared-memory",
    "-C", "link-arg=--import-memory",
    "-C", "link-arg=--export=__wasm_init_memory",

    # 3. Thread-Local Storage (TLS) exports required by lld & wasm-bindgen
    "-C", "link-arg=--export=__wasm_init_tls",
    "-C", "link-arg=--export=__tls_size",
    "-C", "link-arg=--export=__tls_align",
    "-C", "link-arg=--export=__tls_base",

    # 4. Linear memory boundaries (64MB Initial, 2GB Max)
    "-C", "link-arg=--initial-memory=67108864",
    "-C", "link-arg=--max-memory=2147483648",
]

[unstable]
# Recompile core and std with atomics enabled
build-std = ["std", "panic_abort"]
build-std-features = ["panic_immediate_abort"]
```

### `rust-toolchain.toml`

```toml
[toolchain]
channel = "nightly"
components = ["rust-src"]
targets = ["wasm32-unknown-unknown"]
profile = "minimal"
```

---

## 3. Profiles: Throughput Optimization

Database engines require deterministic execution speed, unrolled loops, and cross-crate inlining over raw binary-size reduction.

### `Cargo.toml`

```toml
[package]
name = "wasm_engine"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
rayon = "1.10"
wasm-bindgen-rayon = "1.3"
js-sys = "0.3"
web-sys = { version = "0.3", features = [
    "DedicatedWorkerGlobalScope",
    "FileSystemSyncAccessHandle",
    "FileSystemFileHandle",
    "FileSystemDirectoryHandle",
    "FileSystemGetFileOptions",
    "FileSystemReadWriteOptions",
    "StorageManager"
] }

[profile.release]
opt-level = 3          # Maximize vectorization and loop optimizations
lto = "fat"            # Full link-time optimization across all dependencies
codegen-units = 1      # Single CGU enables global LLVM inlining
panic = "abort"        # Eliminates frame unwind tables
debug = false
strip = "symbols"      # Strips debug symbols while retaining exports
```

---

## 4. Multi-Threading Architecture: The Dedicated Worker Pattern

### The `Atomics.wait` Browser Invariant
Web browsers enforce an absolute security and responsiveness constraint: **`Atomics.wait` cannot be invoked on the main UI/window thread**. 

Because Rayon’s work-stealing scheduler relies on blocking primitives (`Atomics.wait`) to park and coordinate threads during `par_iter`, `join`, or barrier synchronization, **any execution of Rayon on the browser's UI thread triggers an uncatchable runtime panic**:
```text
RuntimeError: Atomics.wait cannot be called in this context
```

### Architectural Layout

To comply with browser constraints, the application must isolate the WebAssembly engine inside a **Dedicated Web Worker**. The dedicated worker manages the Rayon thread pool (spawning auxiliary workers for compute partitions) and executes synchronous file system calls.

```text
┌──────────────────────── Browser UI Thread ────────────────────────┐
│  - Captures DOM events / UI state                                  │
│  - Communicates via MessageChannel / postMessage                  │
└─────────────────────────────────┬─────────────────────────────────┘
                                  │ postMessage({ query })
┌─────────────────────────────────▼─────────────────────────────────┐
│                     Dedicated Worker (DB Host)                    │
│  - Hosts the WebAssembly Memory Instance                          │
│  - Executes synchronous OPFS I/O (FileSystemSyncAccessHandle)     │
│  - Dispatches parallel queries across Rayon pool                  │
│                                                                   │
│       ┌─────────────── Rayon Thread Pool (Workers) ────────┐      │
│       │  [Worker 1]    [Worker 2]    ...    [Worker N]      │      │
│       │  Compute partition scans across SharedArrayBuffer  │      │
│       └─────────────────────────────────────────────────────┘      │
└───────────────────────────────────────────────────────────────────┘
```

### Rust Engine Implementation (`src/lib.rs`)

```rust
use wasm_bindgen::prelude::*;
use rayon::prelude::*;

// Expose Rayon's thread pool initialization entrypoint to JavaScript
pub use wasm_bindgen_rayon::init_thread_pool;

#[wasm_bindgen]
pub struct DatabaseKernel {
    epoch: std::sync::atomic::AtomicU64,
}

#[wasm_bindgen]
impl DatabaseKernel {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            epoch: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Parallel compute kernel executed across the thread pool
    pub fn parallel_scan(&self, records: &[i64], target: i64) -> usize {
        records
            .par_iter()
            .filter(|&&val| val == target)
            .count()
    }
}
```

### Host Worker Setup (`db-worker.js`)

```javascript
import init, { initThreadPool, DatabaseKernel } from './pkg/wasm_engine.js';

let kernel = null;

self.onmessage = async (event) => {
    const { action, payload } = event.data;

    switch (action) {
        case 'BOOTSTRAP': {
            // 1. Initialize the Wasm module instance
            await init();

            // 2. Initialize Rayon thread pool
            // Leave 1 core for the UI thread and 1 for the host worker
            const availableThreads = navigator.hardwareConcurrency || 4;
            const workerThreads = Math.max(1, availableThreads - 1);
            await initThreadPool(workerThreads);

            kernel = new DatabaseKernel();
            self.postMessage({ status: 'READY' });
            break;
        }

        case 'EXECUTE_QUERY': {
            const dataView = payload.data; // BigInt64Array view of shared memory
            const matches = kernel.parallel_scan(dataView, payload.target);
            self.postMessage({ status: 'SUCCESS', result: matches });
            break;
        }
    }
};
```

---

## 5. Storage Engine Design: OPFS & Synchronous VFS Architecture

For transactional databases, asynchronous web storage (IndexedDB, Cache API) is unsuitable due to message-loop latency, non-deterministic flush characteristics, and lack of true atomic writes. The modern web storage standard is the **Origin Private File System (OPFS)** utilized via `FileSystemSyncAccessHandle`.

### Critical OPFS Invariants (Lessons from SQLite Wasm / `wa-sqlite`)

1. **Worker Context Isolation**:
   `createSyncAccessHandle()` is unavailable on the main thread. It exists exclusively inside Dedicated Web Workers, aligning with the dedicated database worker architecture.
2. **Handle Exclusivity (`NoModificationAllowedError`)**:
   Opening an access handle locks the file at the browser/OS level. If another worker, tab, or process attempts to call `createSyncAccessHandle()` on the same file, the call throws `NoModificationAllowedError`.
   * *Architecture Guideline*: Adopt a **Single-Writer Process Model**. A single dedicated worker manages the database files and coordinates all writes. Read-only replicas must coordinate handle acquisition or rely on client-side RPC to the primary database worker.
3. **Flush Costs and Amortization**:
   In OPFS, invoking `.flush()` maps to an OS-level synchronous sync (`fdatasync`/`fsync`). Calling `.flush()` on every row or single-page write degrades throughput by multiple orders of magnitude.
   * *Architecture Guideline*: Implement a **Group Commit** mechanism and a Write-Ahead Log (WAL). Batch multiple dirty page transactions into sequential WAL entries and issue a single `.flush()` per commit interval.
4. **Pre-Allocation & Chunked Extents**:
   Dynamic, byte-by-byte file growth forces the browser to continually update internal metadata tables. 
   * *Architecture Guideline*: Allocate space ahead of time using `.truncate(newSize)` in fixed blocks (e.g., 64 KiB or 1 MiB extents) instead of writing past EOF.
5. **Page Alignment**:
   Align database pages to **4096** or **8192** bytes. This matches both the underlying host OS virtual memory page sizes and browser-internal allocation boundaries.

### OPFS Synchronous VFS Implementation in Rust

```rust
use wasm_bindgen::prelude::*;
use web_sys::{FileSystemFileHandle, FileSystemSyncAccessHandle, FileSystemReadWriteOptions};
use js_sys::Uint8Array;

pub const PAGE_SIZE: usize = 4096;

pub struct OpfsStorageEngine {
    handle: FileSystemSyncAccessHandle,
    scratch_buffer: Vec<u8>,
}

impl OpfsStorageEngine {
    pub fn new(handle: FileSystemSyncAccessHandle) -> Self {
        Self {
            handle,
            scratch_buffer: vec![0u8; PAGE_SIZE],
        }
    }

    /// Read an aligned page directly from a given file offset
    pub fn read_page(&self, page_id: u32, target: &mut [u8; PAGE_SIZE]) -> Result<(), JsValue> {
        let offset = (page_id as f64) * (PAGE_SIZE as f64);
        
        let mut opts = FileSystemReadWriteOptions::new();
        opts.at(offset);

        // Safe projection of target slice as an ArrayBufferView
        let bytes_read = self.handle.read_with_u8_array_and_options(target, &opts)?;
        
        if (bytes_read as usize) < PAGE_SIZE {
            return Err(JsValue::from_str("Partial page read detected"));
        }
        Ok(())
    }

    /// Writes an aligned page using synchronous in-place file mutation
    pub fn write_page(&self, page_id: u32, data: &[u8; PAGE_SIZE]) -> Result<(), JsValue> {
        let offset = (page_id as f64) * (PAGE_SIZE as f64);
        
        let mut opts = FileSystemReadWriteOptions::new();
        opts.at(offset);

        self.handle.write_with_u8_array_and_options(data, &opts)?;
        Ok(())
    }

    /// Explicit synchronization to persistent storage (call during WAL commit)
    pub fn sync(&self) -> Result<(), JsValue> {
        self.handle.flush()
    }

    /// Chunked file pre-allocation to prevent metadata fragmentation
    pub fn ensure_capacity(&self, required_bytes: f64) -> Result<(), JsValue> {
        let current_size = self.handle.get_size()?;
        if current_size < required_bytes {
            // Pad out to the nearest 1MB boundary
            let chunk_size = 1024.0 * 1024.0;
            let target_size = (required_bytes / chunk_size).ceil() * chunk_size;
            self.handle.truncate_with_f64(target_size)?;
        }
        Ok(())
    }
}
```

---

## 6. Hardware Vectorization: WebAssembly SIMD (`simd128`)

Modern browser engines translate WebAssembly 128-bit SIMD instructions into native x86 AVX or ARM Neon instructions.

### Explicit Vector Operations (`core::arch::wasm32`)

For performance-critical loops (filtering, hash calculations, roaring bitmap scans), use explicit intrinsics instead of relying on autovectorization:

```rust
use core::arch::wasm32::*;

#[target_feature(enable = "simd128")]
pub unsafe fn vectorized_column_equals(column: &[i32], needle: i32, out_mask: &mut [u8]) {
    assert_eq!(column.len(), out_mask.len());
    let needle_vec = i32x4_splat(needle);

    let chunks = column.chunks_exact(4);
    let out_chunks = out_mask.chunks_exact_mut(4);
    let remainder = chunks.remainder();
    let remainder_offset = column.len() - remainder.len();

    for (c, o) in chunks.zip(out_chunks) {
        let data = v128_load(c.as_ptr() as *const v128);
        // Returns 0xFFFFFFFF for match, 0x00000000 otherwise
        let cmp = i32x4_eq(data, needle_vec);
        let bitmask = i32x4_bitmask(cmp); // 4-bit integer representing matches

        o[0] = ((bitmask >> 0) & 1) as u8;
        o[1] = ((bitmask >> 1) & 1) as u8;
        o[2] = ((bitmask >> 2) & 1) as u8;
        o[3] = ((bitmask >> 3) & 1) as u8;
    }

    // Process remainder scalar elements
    for (i, &val) in remainder.iter().enumerate() {
        out_mask[remainder_offset + i] = if val == needle { 1 } else { 0 };
    }
}
```

---

## 7. Zero-Copy Linear Memory Architecture

Crossing the JavaScript-WebAssembly boundary using standard arguments (e.g., returning `Vec<T>` or passing `js_sys::Array`) copies data across the boundary via memory allocations. For high-throughput analytics, pass raw memory pointers and lengths, creating typed views on both sides.

### Zero-Copy Memory Pointers in Rust

```rust
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SharedColumn {
    buffer: Vec<i64>,
}

#[wasm_bindgen]
impl SharedColumn {
    #[wasm_bindgen(constructor)]
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0i64; capacity],
        }
    }

    pub fn data_ptr(&self) -> *const i64 {
        self.buffer.as_ptr()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }
}
```

### Direct Buffer Construction in JavaScript

```javascript
function readColumnZeroCopy(wasmInstance, sharedColumn) {
    const memory = wasmInstance.memory; // WebAssembly.Memory instance (SharedArrayBuffer)
    const ptr = sharedColumn.data_ptr();
    const len = sharedColumn.len();

    // Construct view directly pointing to Rust memory space:
    return new BigInt64Array(memory.buffer, ptr, len);
}
```

### Memory Growth Semantics: `ArrayBuffer` vs. `SharedArrayBuffer`
* In single-threaded Wasm, calling `memory.grow()` **detaches** and invalidates all existing typed arrays.
* In multi-threaded Wasm, the underlying backing is a **`SharedArrayBuffer`**, which **cannot be detached**. However, existing `TypedArray` views retain their original `byteLength`. If dynamic heap allocation causes a growth event, recreate your `TypedArray` references to access newly allocated memory ranges beyond the old bound.

---

## 8. Build, Optimization, and Deployment Pipeline

### Local Development via Trunk (`Trunk.toml`)

Trunk manages compilation, wasm-bindgen invocation, and sets required HTTP isolation headers for local development:

```toml
# Trunk.toml
[build]
target = "index.html"

[serve]
addresses = ["127.0.0.1"]
port = 8080

[serve.headers]
# Mandatory security headers for SharedArrayBuffer
"Cross-Origin-Opener-Policy" = "same-origin"
"Cross-Origin-Embedder-Policy" = "require-corp"
```

### Production Release Pipeline (`build.sh`)

Running `wasm-opt` requires explicit flags to inform the optimizer about threading and vector extensions; omitting these flags causes the optimizer to strip or reject SIMD and atomic instructions.

```bash
#!/usr/bin/env bash
set -euo pipefail

DIST="dist"
PKG="$DIST/pkg"
RAW_WASM="target/wasm32-unknown-unknown/release/wasm_engine.wasm"

mkdir -p "$PKG"

# 1. Compile through Cargo with Nightly build-std
cargo build \
    --target wasm32-unknown-unknown \
    --release \
    -Z build-std=std,panic_abort \
    -Z build-std-features=panic_immediate_abort

# 2. Run wasm-bindgen (Target must be 'web')
wasm-bindgen "$RAW_WASM" \
    --out-dir "$PKG" \
    --target web \
    --no-typescript

# 3. Post-Link binary optimization via wasm-opt
# Explicitly preserve instruction subsets
wasm-opt -O3 \
    --enable-threads \
    --enable-bulk-memory \
    --enable-simd \
    --enable-mutable-globals \
    "$PKG/wasm_engine_bg.wasm" \
    -o "$PKG/wasm_engine_bg.wasm"

# 4. Generate pre-compressed production assets
brotli -11 -k -f "$PKG/wasm_engine_bg.wasm"
gzip -9 -k -f "$PKG/wasm_engine_bg.wasm"

echo "Build complete. Artifacts written to: $PKG"
```

### Profiling Static Binary Size with `twiggy`

To diagnose unwanted symbols (such as formatting dependencies or unexpected allocators), profile the unstripped binary:

```bash
# Display top 25 dominators
twiggy top -n 25 target/wasm32-unknown-unknown/release/wasm_engine.wasm

# Trace retention paths for specific formatting logic
twiggy paths target/wasm32-unknown-unknown/release/wasm_engine.wasm "core::fmt::*"
```

---

## 9. Verification & Troubleshooting Reference

| Error / Failure | Root Cause | Solution |
| :--- | :--- | :--- |
| `RuntimeError: Atomics.wait cannot be called in this context` | Rayon iterator or barrier triggered directly on the main UI thread. | Relocate Wasm initialization and query execution into a Dedicated Web Worker. |
| `undefined symbol: __wasm_init_tls` | LLVM `lld` stripped thread-local storage exports. | Add `-C link-arg=--export=__wasm_init_tls` (and associated `__tls_*` symbols) to `.cargo/config.toml`. |
| `NoModificationAllowedError` | Multiple workers or tabs attempted to call `createSyncAccessHandle` on the same OPFS file. | Implement a Single-Writer architecture where one worker owns all file writes. |
| `ReferenceError: SharedArrayBuffer is not defined` | Missing Cross-Origin Isolation headers. | Configure your server to send `COOP: same-origin` and `COEP: require-corp`. |
| `RangeError: WebAssembly.Memory(): could not allocate memory` | Requested linear memory maximum exceeds browser boundary limits. | Reduce `--max-memory` from 4 GiB to 2 GiB (`2147483648` bytes) in `.cargo/config.toml`. |
| `wasm-opt error: unexpected feature "atomics"` | `wasm-opt` executed without multi-threading and feature flags. | Include `--enable-threads --enable-bulk-memory --enable-simd` during the `wasm-opt` pass. |

---

## 10. Specifications and Standard References

- [W3C WebAssembly Threads Specification](https://github.com/WebAssembly/threads)
- [W3C WebAssembly 128-bit SIMD Specification](https://webassembly.github.io/simd/)
- [W3C Origin Private File System: FileSystemSyncAccessHandle](https://fs.spec.whatwg.org/#filesystemsyncaccesshandle)
- [MDN: Cross-Origin Isolation Architecture](https://developer.mozilla.org/en-US/docs/Web/API/crossOriginIsolated)
- [Rust Tracking Issue for `-Z build-std` (#53964)](https://github.com/rust-lang/rust/issues/53964)
- [wasm-bindgen Reference Guide](https://rustwasm.github.io/wasm-bindgen/)
- [SQLite Formal OPFS Implementation Architecture](https://sqlite.org/wasm/doc/trunk/persistence.md)