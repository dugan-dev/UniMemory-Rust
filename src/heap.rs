use crate::{
    backend, memory::dangling, statistics::Tracker, Block, Error, Memory, Result, Statistics,
    StatisticsMode,
};
use std::{alloc::Layout, cell::RefCell, marker::PhantomData, ptr::NonNull, rc::Rc};

struct State {
    native: backend::NativeHeap,
    count: usize,
    #[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
    registry: crate::registry::Registry,
}

/// Independent allocation group, thread-confined consistently across all backends.
/// Native backends use dedicated heaps/arenas; Rust/Standard retain an allocation registry.
///
/// ```compile_fail
/// let heap = unimemory::Heap::new().unwrap();
/// std::thread::spawn(move || drop(heap)); // native heap stays on its owning thread
/// ```
pub struct Heap {
    state: RefCell<State>,
    tracking: Tracker,
    thread: PhantomData<Rc<()>>,
}
impl Heap {
    /// Create a heap with statistics disabled.
    pub fn new() -> Result<Self> {
        Ok(Self {
            state: RefCell::new(State {
                native: backend::NativeHeap::new()?,
                count: 0,
                #[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
                registry: crate::registry::Registry::new(),
            }),
            tracking: Tracker::new(),
            thread: PhantomData,
        })
    }
    /// Create a heap with the selected statistics mode.
    pub fn with_statistics(mode: StatisticsMode) -> Result<Self> {
        let mut result = Self::new()?;
        result.set_statistics(mode)?;
        Ok(result)
    }
    /// Reclaim remaining storage without destructors. Live safe owners exclude this borrow.
    pub fn reset(&mut self) -> Result<()> {
        let state = self.state.get_mut();
        #[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
        state.registry.clear();
        #[cfg(any(feature = "mimalloc", feature = "jemalloc"))]
        state.native.reset()?;
        state.count = 0;
        self.tracking.clear();
        Ok(())
    }
    /// Ask native heaps to release unused pages. Rust/Standard have no equivalent tuning operation.
    pub fn collect(&self) -> Result<()> {
        self.state.borrow().native.collect()
    }
    /// Check a safe block's resource identity without inspecting unvalidated raw addresses.
    pub fn owns(&self, block: &Block<'_, Self>) -> bool {
        std::ptr::eq(self, block.resource())
    }
    /// Outstanding nonzero allocations, including forgotten owners.
    pub fn live_allocations(&self) -> usize {
        self.state.borrow().count
    }
    /// Change tracking while idle; reset forgotten allocations first.
    pub fn set_statistics(&mut self, mode: StatisticsMode) -> Result<()> {
        if self.state.get_mut().count != 0 {
            return Err(Error::Busy);
        }
        self.tracking.set_mode(mode)
    }
    /// Clear counters with no tracked live allocations.
    pub fn reset_statistics(&mut self) -> Result<()> {
        self.tracking.reset()
    }
}

// SAFETY: storage stays owned until matched release or exclusive reset. RefCell/Rc prevent
// concurrent use and native-heap thread migration. Each allocation is disjoint and aligned.
unsafe impl Memory for Heap {
    fn kind(&self) -> crate::MemoryKind {
        crate::MemoryKind::Heap
    }
    #[inline]
    fn allocate_raw(&self, layout: Layout, zeroed: bool) -> Result<NonNull<u8>> {
        if layout.size() == 0 {
            return Ok(dangling(layout));
        }
        let state = &mut *self.state.borrow_mut();
        #[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
        let pointer = state.registry.allocate(layout, zeroed)?;
        #[cfg(any(feature = "mimalloc", feature = "jemalloc"))]
        // SAFETY: live native heap on its owning thread, valid nonzero layout.
        let pointer = backend::checked(unsafe { state.native.allocate(layout, zeroed) })?;
        state.count += 1;
        self.tracking.allocated(pointer.as_ptr(), layout.size());
        Ok(pointer)
    }
    #[inline]
    unsafe fn deallocate_raw(&self, pointer: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        let state = &mut *self.state.borrow_mut();
        #[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
        // SAFETY: caller supplies the exact allocation/layout from this registry.
        unsafe {
            state.registry.deallocate(pointer, layout);
        }
        #[cfg(any(feature = "mimalloc", feature = "jemalloc"))]
        // SAFETY: caller supplies the exact allocation/layout from this native heap.
        unsafe {
            state.native.deallocate(pointer.as_ptr(), layout);
        }
        state.count -= 1;
        self.tracking.deallocated(pointer.as_ptr(), layout.size());
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
            // SAFETY: live original allocation, exact layout, no outstanding references.
            unsafe {
                self.deallocate_raw(pointer, old);
            }
            return Ok(dangling(new));
        }
        let state = &mut *self.state.borrow_mut();
        #[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
        // SAFETY: registry owns the exact original pointer/layout pair.
        let next = unsafe { state.registry.resize(pointer, old, new) }?;
        #[cfg(any(feature = "mimalloc", feature = "jemalloc"))]
        let next = if old.align() == new.align() {
            // SAFETY: matching native allocation, unchanged alignment, valid new size.
            backend::checked(unsafe { state.native.resize(pointer.as_ptr(), old, new) })?
        } else {
            // SAFETY: new is a valid nonzero layout for this native heap.
            let next = backend::checked(unsafe { state.native.allocate(new, false) })?;
            // SAFETY: disjoint live blocks, valid prefix copy, matching original deallocation.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    next.as_ptr(),
                    old.size().min(new.size()),
                );
                state.native.deallocate(pointer.as_ptr(), old);
            }
            next
        };
        if zeroed && new.size() > old.size() {
            // SAFETY: successful growth provides this unique writable extension.
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
