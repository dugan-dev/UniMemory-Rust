# Design and safety / 设计与安全

The Rust frontend is implemented independently. The original C++ UniMemory checkout is never modified. No backend's allocator algorithm is translated or reimplemented.

| C++ UniMemory | Rust frontend |
| --- | --- |
| Global / Heap / Stack | Same public names, backend-independent concrete types |
| Memory | An unsafe implementation trait with a safe `MemoryExt` interface |
| create / destroy | `create(value)` returns `Owned`; Drop destroys and releases |
| create_array / destroy_array | `create_array(count, initializer)` returns `Array` |
| OwnedBlock | `Block`, with initialized/uninitialized byte views distinguished |
| allocator / PMR | Stable allocator-api2 adapter; generic by default, `&dyn Memory` optional |
| statistics | Same five fields; optional compiled support and per-object mode |
| config(milliseconds) | `Global::config` or `unimemory::config` |
| owns(raw pointer) | `Heap::owns(&block)` checks resource identity without probing invalid addresses |
| reset / rewind | Require `&mut self`, so borrowing safe owners prevent early reclamation |

## Ownership

Resources own or borrow raw storage. `Block`, `Owned`, `Array` and container allocators borrow a resource. Their borrow prevents destruction, movement and exclusive reset while they remain in use. `Heap` and `Stack` are thread-confined; `Global` and owners with appropriate Send/Sync bounds support cross-thread use.

Raw `allocate_raw`, `deallocate_raw` and `reallocate_raw` follow an explicit unsafe deallocation contract. Dereferencing a raw pointer, bypassing borrow tracking, or using a pointer after reset/rewind requires the caller to maintain validity. Zero-sized layouts return aligned dangling pointers without invoking native allocators or updating statistics.

Reset/rewind never run destructors. Safe Rust normally drops owners first; forgetting an owner permits reclamation but also intentionally skips its destructor. A Stack checkpoint is tied to an instance and generation. Rewind invalidates previous checkpoints and storage crossing or following its saved position. The buffer remains exclusively borrowed for the Stack's lifetime. Stack accepts `&mut [MaybeUninit<u8>]`, so uninitialized data and object padding never get returned to a safe ordinary-byte buffer. Mutable raw-pointer access invalidates Block's initialized-byte flag; external writers must explicitly prove full initialization before restoring byte views.

## Implementations

Rust Global delegates to `std::alloc::{alloc,dealloc,realloc}` and therefore honors the application's global allocator. Standard uses a small C++17 `extern "C"` bridge to matched `operator new/delete`; exceptions never cross the ABI. mimalloc and jemalloc use their fixed native APIs directly.

Native Heap uses a real mimalloc heap or an explicit jemalloc arena with thread caches bypassed. Rust/Standard do not offer an equivalent native group; their Heap keeps up to eight records inline and uses a reusable private hash map for overflow, retaining original pointer provenance for bulk cleanup. This service necessarily adds work compared with one plain allocation. The registry reuses capacity and uses a pointer-specific internal hasher; it does not hash untrusted application keys.

Recovering a prefixed header through an allocator-aware Box's payload pointer can violate Rust's borrow model. The implementation retains original pointers in an independent registry instead. This was exercised with strict-provenance Miri tests.

Statistics are opt-in. Disabled counters perform no map insertion or locking, and compiling without `statistics` removes tracking storage. Basic mode serializes requested-byte counters and live allocation records. Mode changes preserve history and require idle tracked allocations; reset clears history. Nested allocations inside a String/Vec field do not automatically belong to its outer object's resource.

## Deliberate Rust differences

Standard library per-container Allocator APIs are not assumed stable. Allocator-aware containers are re-exported from allocator-api2. Standard containers and Arc can use the selected backend globally through `NativeAllocator`; a per-resource shared-pointer implementation is not claimed. Registering NativeAllocator works unchanged in all profiles; its Rust-mode GlobalAlloc implementation calls Rust System directly to avoid recursion. Without this explicit registration, Rust-mode resources honor the application's existing global allocator, including externally supplied mimalloc.

Default typed-resource calls use static dispatch and can inline. An explicit `&dyn Memory` permits one resource-independent container/owner type, at the expected dynamic dispatch cost. The public Global/Heap/Stack types do not depend on the backend feature.

Backend features are mutually exclusive and intentionally fail to compile when combined. Transitive Cargo features must agree within an application. FFI allocations are always freed by matching APIs; even Rust System and C++ new/delete are not interchangeable.

## Reproduce checks

```sh
python tools/validate.py
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test --lib --tests
```

Miri targets the Rust backend and interprets a Linux target in local Windows validation. Native FFI backends are checked with ordinary Debug/Release tests; Miri cannot execute their C allocator implementations. Local test counts and measured performance are recorded separately; cross-platform CI configuration is not itself proof that those remote machines passed.
