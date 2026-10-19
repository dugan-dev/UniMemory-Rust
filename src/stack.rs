use crate::{
    memory::dangling, statistics::Tracker, Error, Memory, Result, Statistics, StatisticsMode,
};
use std::{
    alloc::Layout,
    cell::Cell,
    marker::PhantomData,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Opaque checkpoint, valid only for its originating Stack and current generation.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    owner: u64,
    generation: u64,
    used: usize,
}

/// Thread-confined bump allocator borrowing a fixed buffer. Individual frees do not reclaim space.
///
/// ```compile_fail
/// use unimemory::{MemoryExt, Stack};
/// use std::mem::MaybeUninit;
/// let mut buffer = [MaybeUninit::<u8>::uninit(); 128];
/// let stack = Stack::new(&mut buffer);
/// let value = stack.create(42).unwrap();
/// buffer[0].write(0); // buffer remains exclusively borrowed
/// assert_eq!(*value, 42);
/// ```
///
/// ```compile_fail
/// let mut initialized_bytes = [0u8; 128];
/// let stack = unimemory::Stack::new(&mut initialized_bytes);
/// // Ordinary bytes cannot safely receive arbitrary uninitialized data/padding.
/// ```
pub struct Stack<'buffer> {
    buffer: NonNull<u8>,
    capacity: usize,
    used: Cell<usize>,
    id: u64,
    generation: u64,
    tracking: Tracker,
    borrow: PhantomData<&'buffer mut [MaybeUninit<u8>]>,
}

impl<'buffer> Stack<'buffer> {
    /// Borrow a fixed buffer. The buffer may originate from the stack, heap or elsewhere.
    pub fn new(buffer: &'buffer mut [MaybeUninit<u8>]) -> Self {
        let mut current = NEXT_ID.load(Ordering::Relaxed);
        let id = loop {
            let next = current
                .checked_add(1)
                .expect("Stack identity space exhausted");
            match NEXT_ID.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break current,
                Err(observed) => current = observed,
            }
        };
        Self {
            buffer: NonNull::new(buffer.as_mut_ptr().cast()).expect("slice pointer is nonnull"),
            capacity: buffer.len(),
            used: Cell::new(0),
            id,
            generation: 0,
            tracking: Tracker::new(),
            borrow: PhantomData,
        }
    }
    /// Borrow a fixed buffer with optional allocation statistics.
    pub fn with_statistics(
        buffer: &'buffer mut [MaybeUninit<u8>],
        mode: StatisticsMode,
    ) -> Result<Self> {
        let mut result = Self::new(buffer);
        result.set_statistics(mode)?;
        Ok(result)
    }
    /// Consumed buffer bytes, including alignment padding and individually released blocks.
    pub fn used(&self) -> usize {
        self.used.get()
    }
    /// Total borrowed buffer size.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Save the current bump position.
    pub fn mark(&self) -> Mark {
        Mark {
            owner: self.id,
            generation: self.generation,
            used: self.used(),
        }
    }
    /// Reclaim bytes after a checkpoint; no destructors run. Invalidates previous checkpoints.
    pub fn rewind(&mut self, mark: Mark) -> Result<()> {
        if mark.owner != self.id || mark.generation != self.generation || mark.used > self.used() {
            return Err(Error::InvalidMark);
        }
        let next = self.generation.checked_add(1).ok_or(Error::InvalidMark)?;
        self.tracking
            .rewind(self.buffer.as_ptr().addr() + mark.used);
        self.used.set(mark.used);
        self.generation = next;
        Ok(())
    }
    /// Reclaim the whole buffer, without running destructors. Safe owners prevent this borrow.
    pub fn reset(&mut self) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or(Error::InvalidMark)?;
        self.used.set(0);
        self.tracking.clear();
        Ok(())
    }
    /// Change tracking mode when no tracked allocations remain.
    pub fn set_statistics(&mut self, mode: StatisticsMode) -> Result<()> {
        self.tracking.set_mode(mode)
    }
    /// Clear counters when no tracked allocations remain.
    pub fn reset_statistics(&mut self) -> Result<()> {
        self.tracking.reset()
    }

    fn reserve(&self, layout: Layout) -> Result<NonNull<u8>> {
        if layout.size() == 0 {
            return Ok(dangling(layout));
        }
        let used = self.used();
        let address = self.buffer.as_ptr().addr() + used;
        let padding = address.wrapping_neg() & (layout.align() - 1);
        let end = used
            .checked_add(padding)
            .and_then(|n| n.checked_add(layout.size()))
            .ok_or(Error::OutOfMemory)?;
        if end > self.capacity {
            return Err(Error::OutOfMemory);
        }
        // SAFETY: end is within the original buffer; this disjoint bump range retains its provenance.
        let pointer = unsafe { NonNull::new_unchecked(self.buffer.as_ptr().add(used + padding)) };
        self.used.set(end);
        Ok(pointer)
    }
}

// SAFETY: allocation ranges are disjoint and stable in the borrowed buffer. Cell prevents sharing;
// buffer ownership and exclusive rewind/reset prevent safe references from being invalidated.
unsafe impl Memory for Stack<'_> {
    fn kind(&self) -> crate::MemoryKind {
        crate::MemoryKind::Stack
    }
    fn allocate_raw(&self, layout: Layout, zeroed: bool) -> Result<NonNull<u8>> {
        let pointer = self.reserve(layout)?;
        if zeroed {
            // SAFETY: reserve provided uniquely owned storage for precisely this many bytes.
            unsafe {
                pointer.as_ptr().write_bytes(0, layout.size());
            }
        }
        self.tracking.allocated(pointer.as_ptr(), layout.size());
        Ok(pointer)
    }
    unsafe fn deallocate_raw(&self, pointer: NonNull<u8>, layout: Layout) {
        self.tracking.deallocated(pointer.as_ptr(), layout.size());
    }
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
            // SAFETY: original live pointer and layout satisfy deallocation's contract.
            unsafe {
                self.deallocate_raw(pointer, old);
            }
            return Ok(dangling(new));
        }
        let aligned = pointer.as_ptr().addr() & (new.align() - 1) == 0;
        let used = self.used();
        let last = pointer.as_ptr().addr().checked_add(old.size())
            == Some(self.buffer.as_ptr().addr() + used);
        let next = if aligned && new.size() <= old.size() {
            pointer
        } else if aligned && last && new.size() - old.size() <= self.capacity - used {
            self.used.set(used + new.size() - old.size());
            pointer
        } else {
            let next = self.reserve(new)?;
            // SAFETY: fresh bump allocation is disjoint; both blocks fit the copied byte count.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    next.as_ptr(),
                    old.size().min(new.size()),
                );
            }
            next
        };
        if zeroed && new.size() > old.size() {
            // SAFETY: successful growth provides these additional bytes.
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
