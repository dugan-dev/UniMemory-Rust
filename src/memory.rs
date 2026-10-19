use crate::{Array, Block, Owned, Result, Statistics, StatisticsMode};
use std::{alloc::Layout, ptr::NonNull};

/// Resource lifetime strategy, independent of the selected native backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    /// Shared native or application allocator.
    Global,
    /// Independently reclaimable allocation group.
    Heap,
    /// Borrowed fixed-capacity buffer.
    Stack,
}

/// A memory resource. Safe ownership helpers are provided by [`MemoryExt`].
///
/// # Safety
/// Implementations must return disjoint, correctly aligned allocations, keep them
/// valid until explicitly released or an exclusively borrowed reset, and preserve
/// the original allocation on a failed reallocation. Deallocation must not unwind.
/// Storage must permit arbitrary uninitialized bytes and typed-object padding;
/// releasing the resource must not invalidate the backing owner's type invariants.
pub unsafe trait Memory {
    /// The resource's lifetime strategy.
    fn kind(&self) -> MemoryKind;
    /// Allocate raw storage. Zero-size layouts return an aligned dangling pointer.
    fn allocate_raw(&self, layout: Layout, zeroed: bool) -> Result<NonNull<u8>>;

    /// Release an allocation made by this resource.
    ///
    /// # Safety
    /// `pointer` must be a live allocation from this resource with exactly `layout`.
    /// It must have no remaining references or owners. Zero-size pointers are ignored.
    unsafe fn deallocate_raw(&self, pointer: NonNull<u8>, layout: Layout);

    /// Resize raw storage, preserving the smaller of the old and new sizes.
    ///
    /// # Safety
    /// `pointer` must be a live allocation with exactly `old`, with no references
    /// into it. On success use only the returned pointer and `new` layout. A reset
    /// or rewind must not have invalidated the original allocation.
    unsafe fn reallocate_raw(
        &self,
        pointer: NonNull<u8>,
        old: Layout,
        new: Layout,
        zeroed: bool,
    ) -> Result<NonNull<u8>>;

    /// Current per-resource statistics; disabled resources return zeroes.
    fn statistics(&self) -> Statistics;
    /// Current tracking mode.
    fn statistics_mode(&self) -> StatisticsMode;
}

/// Safe ownership and stable container adapters for any memory resource.
pub trait MemoryExt: Memory {
    /// Allocate uninitialized bytes with an owner tied to this resource.
    fn allocate(&self, layout: Layout) -> Result<Block<'_, Self>> {
        Block::new_in(self, layout, false)
    }
    /// Allocate initialized zero bytes.
    fn allocate_zeroed(&self, layout: Layout) -> Result<Block<'_, Self>> {
        Block::new_in(self, layout, true)
    }
    /// Store a value; dropping the owner runs its destructor and frees storage.
    fn create<T>(&self, value: T) -> Result<Owned<'_, T, Self>> {
        Owned::new_in(value, self)
    }
    /// Construct an array, rolling back initialized elements if the closure panics.
    fn create_array<T>(
        &self,
        count: usize,
        init: impl FnMut(usize) -> T,
    ) -> Result<Array<'_, T, Self>> {
        Array::new_in(count, init, self)
    }
    /// A stable allocator-api2 adapter borrowing this resource.
    #[cfg(feature = "containers")]
    fn allocator(&self) -> crate::Adapter<'_, Self> {
        crate::Adapter::new(self)
    }
}

impl<M: Memory + ?Sized> MemoryExt for M {}

#[inline]
pub(crate) fn dangling(layout: Layout) -> NonNull<u8> {
    // SAFETY: Layout alignment is nonzero. This address is never dereferenced for ZSTs.
    unsafe { NonNull::new_unchecked(std::ptr::without_provenance_mut(layout.align())) }
}
