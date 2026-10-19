use crate::Memory;
use allocator_api2::alloc::{AllocError, Allocator};
use std::{alloc::Layout, ptr::NonNull};

/// Stable allocator-aware box. Use `Box::new_in(value, memory.allocator())`.
pub use allocator_api2::boxed::Box;
/// Stable allocator-aware vector. Use `Vec::new_in(memory.allocator())`.
pub use allocator_api2::vec::Vec;

/// Borrowing allocator adapter. Its lifetime keeps the resource alive and excludes resets.
///
/// ```compile_fail
/// use unimemory::{Heap, MemoryExt, Vec};
/// let mut heap = Heap::new().unwrap();
/// let mut values = Vec::new_in(heap.allocator());
/// values.push(42);
/// heap.reset().unwrap();
/// assert_eq!(values[0], 42);
/// ```
pub struct Adapter<'a, M: Memory + ?Sized> {
    memory: &'a M,
}
impl<'a, M: Memory + ?Sized> Adapter<'a, M> {
    /// Borrow a memory resource.
    pub fn new(memory: &'a M) -> Self {
        Self { memory }
    }
}
impl<M: Memory + ?Sized> Copy for Adapter<'_, M> {}
impl<M: Memory + ?Sized> Clone for Adapter<'_, M> {
    fn clone(&self) -> Self {
        *self
    }
}

// SAFETY: the borrowed resource outlives all allocations; adapter copies share the same resource.
// Exactly the requested allocation size is reported, so fitting layouts equal their original sizes.
unsafe impl<M: Memory + ?Sized> Allocator for Adapter<'_, M> {
    fn allocate(&self, layout: Layout) -> std::result::Result<NonNull<[u8]>, AllocError> {
        self.memory
            .allocate_raw(layout, false)
            .map(|p| NonNull::slice_from_raw_parts(p, layout.size()))
            .map_err(|_| AllocError)
    }
    fn allocate_zeroed(&self, layout: Layout) -> std::result::Result<NonNull<[u8]>, AllocError> {
        self.memory
            .allocate_raw(layout, true)
            .map(|p| NonNull::slice_from_raw_parts(p, layout.size()))
            .map_err(|_| AllocError)
    }
    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        // SAFETY: Allocator's caller guarantees live allocation identity and a fitting layout.
        unsafe {
            self.memory.deallocate_raw(pointer, layout);
        }
    }
    unsafe fn grow(
        &self,
        pointer: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> std::result::Result<NonNull<[u8]>, AllocError> {
        // SAFETY: forwarded Allocator grow contract, with exact reported allocation sizes.
        unsafe { self.memory.reallocate_raw(pointer, old, new, false) }
            .map(|p| NonNull::slice_from_raw_parts(p, new.size()))
            .map_err(|_| AllocError)
    }
    unsafe fn grow_zeroed(
        &self,
        pointer: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> std::result::Result<NonNull<[u8]>, AllocError> {
        // SAFETY: forwarded Allocator grow_zeroed contract; Memory clears the newly added range.
        unsafe { self.memory.reallocate_raw(pointer, old, new, true) }
            .map(|p| NonNull::slice_from_raw_parts(p, new.size()))
            .map_err(|_| AllocError)
    }
    unsafe fn shrink(
        &self,
        pointer: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> std::result::Result<NonNull<[u8]>, AllocError> {
        // SAFETY: forwarded Allocator shrink contract, including zero-size destination layouts.
        unsafe { self.memory.reallocate_raw(pointer, old, new, false) }
            .map(|p| NonNull::slice_from_raw_parts(p, new.size()))
            .map_err(|_| AllocError)
    }
}
