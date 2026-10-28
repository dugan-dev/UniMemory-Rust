//! Unified memory resources for Rust.
//!
//! Choose `standard` (default), `mimalloc` or `jemalloc` at compile time. Memory
//! owners borrow their resource, so safe code cannot reset it while it is used.
//!
//! ```
//! use unimemory::{Global, MemoryExt};
//! let memory = Global::new();
//! let answer = memory.create(42).unwrap();
//! assert_eq!(*answer, 42);
//! ```
//!
//! ```compile_fail
//! use unimemory::{Heap, MemoryExt};
//! let mut heap = Heap::new().unwrap();
//! let value = heap.create(42).unwrap();
//! heap.reset().unwrap(); // value still borrows heap
//! assert_eq!(*value, 42);
//! ```

#[cfg(any(
    all(feature = "standard", any(feature = "mimalloc", feature = "jemalloc")),
    all(feature = "mimalloc", feature = "jemalloc")
))]
compile_error!("Select exactly one backend: standard, mimalloc OR jemalloc");

mod backend;
mod block;
mod config;
#[cfg(feature = "containers")]
mod containers;
mod error;
mod global;
mod heap;
mod memory;
mod owned;
#[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
mod registry;
mod stack;
mod statistics;

pub use block::Block;
pub use config::config;
#[cfg(feature = "containers")]
pub use containers::{Adapter, Box, Vec};
pub use error::{Error, Result};
pub use global::{Global, NativeAllocator};
pub use heap::Heap;
pub use memory::{Memory, MemoryExt, MemoryKind};
pub use owned::{Array, Owned};
pub use stack::{Mark, Stack};
pub use statistics::{Statistics, StatisticsMode};

/// The allocator selected for this build; Stack always uses its borrowed buffer.
pub const BACKEND: &str = backend::NAME;

/// Native controls for the same-binary benchmark; excluded from normal builds.
#[cfg(feature = "benchmark")]
#[doc(hidden)]
pub mod benchmark {
    use crate::{backend, Memory, Result, Statistics, StatisticsMode};
    use std::{alloc::Layout, ptr::NonNull};

    /// Direct selected-backend controls; unlike a global-allocator provider,
    /// Rust controls deliberately call the application's current global allocator.
    pub struct RawAllocator;
    impl RawAllocator {
        /// Direct allocation.
        /// # Safety
        /// Layout must be nonzero; pair with this same control's deallocation.
        #[inline]
        pub unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: caller guarantees a valid nonzero layout.
            unsafe { backend::allocate(layout, false) }
        }
        /// Direct zeroed allocation.
        /// # Safety
        /// Layout must be nonzero; pair with this same control's deallocation.
        #[inline]
        pub unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: caller guarantees a valid nonzero layout.
            unsafe { backend::allocate(layout, true) }
        }
        /// Direct deallocation.
        /// # Safety
        /// Pointer/layout must match a live allocation from this control, with no views.
        #[inline]
        pub unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: caller supplies the exact matching allocation and layout.
            unsafe {
                backend::deallocate(pointer, layout);
            }
        }
        /// Direct resizing, leaving the original block valid on null.
        /// # Safety
        /// Pointer/layout must match; the nonzero new size must form a valid layout.
        #[inline]
        pub unsafe fn realloc(&self, pointer: *mut u8, old: Layout, size: usize) -> *mut u8 {
            // SAFETY: new size/alignment validity is part of the caller's contract.
            let new = unsafe { Layout::from_size_align_unchecked(size, old.align()) };
            // SAFETY: caller guarantees original ownership and a valid nonzero new layout.
            unsafe { backend::resize(pointer, old, new) }
        }
    }

    /// Direct native heap without the frontend's ownership registry or statistics.
    pub struct NativeHeap(backend::NativeHeap);
    impl NativeHeap {
        /// Create a native heap control on its owning thread.
        pub fn new() -> Result<Self> {
            backend::NativeHeap::new().map(Self)
        }
    }
    // SAFETY: matches native allocation pairs and explicitly supports zero sizes.
    unsafe impl Memory for NativeHeap {
        fn kind(&self) -> crate::MemoryKind {
            crate::MemoryKind::Heap
        }
        #[inline]
        fn allocate_raw(&self, layout: Layout, zeroed: bool) -> Result<NonNull<u8>> {
            if layout.size() == 0 {
                return Ok(crate::memory::dangling(layout));
            }
            // SAFETY: live native heap, valid nonzero layout, owning thread only.
            backend::checked(unsafe { self.0.allocate(layout, zeroed) })
        }
        #[inline]
        unsafe fn deallocate_raw(&self, pointer: NonNull<u8>, layout: Layout) {
            if layout.size() == 0 {
                return;
            }
            // SAFETY: caller guarantees matching ownership and layout.
            unsafe {
                self.0.deallocate(pointer.as_ptr(), layout);
            }
        }
        unsafe fn reallocate_raw(
            &self,
            pointer: NonNull<u8>,
            old: Layout,
            new: Layout,
            zeroed: bool,
        ) -> Result<NonNull<u8>> {
            let next = self.allocate_raw(new, zeroed)?;
            // SAFETY: disjoint blocks, valid copied range, exact old layout.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    next.as_ptr(),
                    old.size().min(new.size()),
                );
                self.deallocate_raw(pointer, old);
            }
            Ok(next)
        }
        fn statistics(&self) -> Statistics {
            Statistics::default()
        }
        fn statistics_mode(&self) -> StatisticsMode {
            StatisticsMode::Disabled
        }
    }
}
