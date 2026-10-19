use crate::{Error, Global, Memory, Result};
use std::{alloc::Layout, mem::MaybeUninit, ptr::NonNull};

/// Unique raw storage borrowing its resource. Uninitialized bytes are never exposed as `u8`.
pub struct Block<'a, M: Memory + ?Sized = Global> {
    pointer: NonNull<u8>,
    layout: Layout,
    memory: &'a M,
    initialized: bool,
}

impl<'a, M: Memory + ?Sized> Block<'a, M> {
    pub(crate) fn new_in(memory: &'a M, layout: Layout, zeroed: bool) -> Result<Self> {
        Ok(Self {
            pointer: memory.allocate_raw(layout, zeroed)?,
            layout,
            memory,
            initialized: zeroed || layout.size() == 0,
        })
    }
    /// Requested byte length.
    pub fn len(&self) -> usize {
        self.layout.size()
    }
    /// Whether the allocation has zero length.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Allocation size and alignment.
    pub fn layout(&self) -> Layout {
        self.layout
    }
    /// The resource borrowed by this allocation.
    pub fn resource(&self) -> &'a M {
        self.memory
    }
    /// Read-only raw address. Do not mutate through it; use [`Self::as_mut_ptr`].
    pub fn as_ptr(&self) -> *const u8 {
        self.pointer.as_ptr()
    }
    /// Unique raw address. Invalidates initialized-byte views until fill or assume_initialized.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.initialized = false;
        self.pointer.as_ptr()
    }
    /// Mark all bytes initialized after an external writer has filled the allocation.
    ///
    /// # Safety
    /// Every byte must be initialized, including padding and trailing storage. Escaped
    /// pointers must not invalidate this guarantee while initialized views can be used.
    pub unsafe fn assume_initialized(&mut self) {
        self.initialized = true;
    }
    /// View bytes without assuming they have been initialized.
    pub fn as_uninit_slice(&self) -> &[MaybeUninit<u8>] {
        // SAFETY: the allocation is live, aligned for bytes and borrowed immutably.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr().cast(), self.len()) }
    }
    /// Mutate storage without initializing it. Invalidates the initialized-byte view.
    pub fn as_uninit_slice_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        self.initialized = false;
        // SAFETY: &mut self gives exclusive access to this allocation.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr().cast(), self.len()) }
    }
    /// Initialized bytes, available after zeroed allocation or [`Self::fill`].
    pub fn bytes(&self) -> Option<&[u8]> {
        self.initialized.then(|| {
            // SAFETY: the flag is set only after every byte is initialized.
            unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.len()) }
        })
    }
    /// Exclusive initialized-byte view.
    pub fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        if !self.initialized {
            return None;
        }
        // SAFETY: every byte is initialized and &mut self is exclusive.
        Some(unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.len()) })
    }
    /// Initialize all bytes to `value`.
    pub fn fill(&mut self, value: u8) {
        // SAFETY: the entire live allocation is writable and exclusively borrowed.
        unsafe {
            self.pointer.as_ptr().write_bytes(value, self.len());
        }
        self.initialized = true;
    }
    /// Resize, leaving any newly added bytes uninitialized. Failure preserves the block.
    pub fn resize(&mut self, bytes: usize) -> Result<()> {
        self.resize_impl(bytes, false)
    }
    /// Resize and initialize newly added bytes to zero. Existing uninitialized bytes remain so.
    pub fn resize_zeroed(&mut self, bytes: usize) -> Result<()> {
        self.resize_impl(bytes, true)
    }
    fn resize_impl(&mut self, bytes: usize, zeroed: bool) -> Result<()> {
        let next = Layout::from_size_align(bytes, self.layout.align())
            .map_err(|_| Error::InvalidLayout)?;
        // SAFETY: this unique owner holds the exact original layout; &mut self excludes views.
        let pointer = unsafe {
            self.memory
                .reallocate_raw(self.pointer, self.layout, next, zeroed)?
        };
        self.initialized = bytes == 0 || (self.initialized && (bytes <= self.len() || zeroed));
        self.pointer = pointer;
        self.layout = next;
        Ok(())
    }
}

impl<M: Memory + ?Sized> Drop for Block<'_, M> {
    fn drop(&mut self) {
        // SAFETY: Block uniquely owns the allocation and retains its exact layout/resource.
        unsafe {
            self.memory.deallocate_raw(self.pointer, self.layout);
        }
    }
}

// SAFETY: ownership is unique; moving it is safe when the resource supports cross-thread access.
unsafe impl<M: Memory + Sync + ?Sized> Send for Block<'_, M> {}
// SAFETY: shared access exposes only immutable slices; mutation requires an exclusive borrow.
unsafe impl<M: Memory + Sync + ?Sized> Sync for Block<'_, M> {}
