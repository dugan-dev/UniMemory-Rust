use crate::{
    backend, memory::dangling, statistics::Tracker, Memory, Result, Statistics, StatisticsMode,
};
use std::{alloc::Layout, ptr::NonNull};

/// Shared native allocator with independent per-object statistics. Thread-safe.
pub struct Global {
    tracking: Tracker,
}

impl Global {
    /// Process-wide unused-page release policy; equivalent to [`crate::config`].
    pub fn config(release_delay_ms: i64) -> Result<()> {
        crate::config(release_delay_ms)
    }
    /// An allocation resource with statistics disabled.
    pub const fn new() -> Self {
        Self {
            tracking: Tracker::new(),
        }
    }
    /// An allocation resource with a selected tracking mode.
    pub fn with_statistics(mode: StatisticsMode) -> Result<Self> {
        let mut result = Self::new();
        result.set_statistics(mode)?;
        Ok(result)
    }
    /// Change tracking mode while exclusively borrowed and without tracked live blocks.
    pub fn set_statistics(&mut self, mode: StatisticsMode) -> Result<()> {
        self.tracking.set_mode(mode)
    }
    /// Clear counters while there are no tracked live blocks.
    pub fn reset_statistics(&mut self) -> Result<()> {
        self.tracking.reset()
    }
}
impl Default for Global {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: delegates allocation identity/layout to one fixed backend; shared statistics are synchronized.
unsafe impl Memory for Global {
    fn kind(&self) -> crate::MemoryKind {
        crate::MemoryKind::Global
    }
    #[inline]
    fn allocate_raw(&self, layout: Layout, zeroed: bool) -> Result<NonNull<u8>> {
        if layout.size() == 0 {
            return Ok(dangling(layout));
        }
        // SAFETY: Layout validates size/alignment and the size is nonzero.
        let pointer = backend::checked(unsafe { backend::allocate(layout, zeroed) })?;
        self.tracking.allocated(pointer.as_ptr(), layout.size());
        Ok(pointer)
    }
    #[inline]
    unsafe fn deallocate_raw(&self, pointer: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        self.tracking.deallocated(pointer.as_ptr(), layout.size());
        // SAFETY: caller supplies the original pointer/layout pair with no remaining owners.
        unsafe {
            backend::deallocate(pointer.as_ptr(), layout);
        }
    }
    #[inline]
    unsafe fn reallocate_raw(
        &self,
        pointer: NonNull<u8>,
        old: Layout,
        new: Layout,
        zeroed: bool,
    ) -> Result<NonNull<u8>> {
        if old.size() == 0 {
            return self.allocate_raw(new, zeroed);
        }
        if new.size() == 0 {
            // SAFETY: forwarded original pointer/layout and exclusive caller ownership.
            unsafe {
                self.deallocate_raw(pointer, old);
            }
            return Ok(dangling(new));
        }
        let next = if old.align() == new.align() {
            // SAFETY: same alignment, live original allocation and valid nonzero sizes.
            backend::checked(unsafe { backend::resize(pointer.as_ptr(), old, new) })?
        } else {
            // SAFETY: new is a valid nonzero layout.
            let next = backend::checked(unsafe { backend::allocate(new, false) })?;
            // SAFETY: disjoint allocations, enough storage in both, old pointer still live.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    next.as_ptr(),
                    old.size().min(new.size()),
                );
                backend::deallocate(pointer.as_ptr(), old);
            }
            next
        };
        if zeroed && new.size() > old.size() {
            // SAFETY: the newly grown range is in bounds and exclusively owned.
            unsafe {
                next.as_ptr()
                    .add(old.size())
                    .write_bytes(0, new.size() - old.size());
            }
        }
        self.tracking
            .resized(pointer.as_ptr(), next.as_ptr(), new.size());
        Ok(next)
    }
    fn statistics(&self) -> Statistics {
        self.tracking.snapshot()
    }
    fn statistics_mode(&self) -> StatisticsMode {
        self.tracking.mode()
    }
}

/// Zero-state direct backend adapter, without per-resource tracking.
/// Implements `GlobalAlloc` for every backend. When explicitly registered in Rust
/// mode, it delegates to Rust System to avoid recursion. Ordinary Rust-mode resources
/// continue to honor whatever global allocator the application has already registered.
pub struct NativeAllocator;

// SAFETY: all GlobalAlloc requests are forwarded to matching native functions;
// failures return null and none of these paths allocate Rust metadata or unwind.
#[cfg(any(feature = "standard", feature = "mimalloc", feature = "jemalloc"))]
unsafe impl std::alloc::GlobalAlloc for NativeAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller guarantees a valid nonzero layout.
        unsafe { backend::allocate(layout, false) }
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller guarantees a valid nonzero layout.
        unsafe { backend::allocate(layout, true) }
    }
    #[inline]
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: GlobalAlloc's caller guarantees matching allocation and layout.
        unsafe {
            backend::deallocate(pointer, layout);
        }
    }
    #[inline]
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller guarantees new_size > 0 and a valid resulting layout.
        let new = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: same allocation, alignment and valid nonzero sizes as required by backend.
        unsafe { backend::resize(pointer, layout, new) }
    }
}

// SAFETY: explicit Rust-mode global registration must not call the global allocator again.
// Forwarding every matching allocation pair directly to System avoids recursion and metadata.
#[cfg(not(any(feature = "standard", feature = "mimalloc", feature = "jemalloc")))]
unsafe impl std::alloc::GlobalAlloc for NativeAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller supplies GlobalAlloc's valid nonzero layout.
        unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller supplies GlobalAlloc's valid nonzero layout.
        unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, layout) }
    }
    #[inline]
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: caller supplies the matching System allocation and exact layout.
        unsafe {
            std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout);
        }
    }
    #[inline]
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: matching System allocation, no aliases and a valid nonzero replacement size.
        unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, pointer, layout, new_size) }
    }
}
