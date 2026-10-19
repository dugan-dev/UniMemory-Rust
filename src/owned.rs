use crate::{Block, Error, Global, Memory, Result};
use std::{
    alloc::Layout,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

/// A value and its storage, both released on Drop, borrowing the resource.
pub struct Owned<'a, T, M: Memory + ?Sized = Global> {
    pointer: NonNull<T>,
    block: Block<'a, M>,
    value: PhantomData<T>,
}

impl<'a, T, M: Memory + ?Sized> Owned<'a, T, M> {
    /// Move a value into memory allocated by `memory`.
    pub fn new_in(value: T, memory: &'a M) -> Result<Self> {
        let mut block = Block::new_in(memory, Layout::new::<T>(), false)?;
        // SAFETY: the block has precisely T's size/alignment and unique uninitialized storage.
        let pointer = unsafe { NonNull::new_unchecked(block.as_mut_ptr().cast::<T>()) };
        // SAFETY: unique, correctly aligned storage is large enough for T, including a ZST.
        unsafe {
            pointer.as_ptr().write(value);
        }
        Ok(Self {
            pointer,
            block,
            value: PhantomData,
        })
    }
    /// Move the value out and immediately release its allocation.
    pub fn into_inner(self) -> T {
        let this = ManuallyDrop::new(self);
        // SAFETY: suppressing Owned's Drop allows reading T exactly once.
        let value = unsafe { this.pointer.as_ptr().read() };
        // SAFETY: move Block out exactly once so its storage is released, without dropping T again.
        drop(unsafe { std::ptr::read(&this.block) });
        value
    }
}
impl<T, M: Memory + ?Sized> Deref for Owned<'_, T, M> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: initialized T stays live until this owner is dropped.
        unsafe { self.pointer.as_ref() }
    }
}
impl<T, M: Memory + ?Sized> DerefMut for Owned<'_, T, M> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: &mut self uniquely borrows this value.
        unsafe { self.pointer.as_mut() }
    }
}
impl<T, M: Memory + ?Sized> Drop for Owned<'_, T, M> {
    fn drop(&mut self) {
        // SAFETY: T was initialized exactly once. Block is dropped afterwards, even if T panics.
        unsafe {
            self.pointer.as_ptr().drop_in_place();
        }
    }
}
// SAFETY: transferring ownership requires a sendable T and a thread-safe resource.
unsafe impl<T: Send, M: Memory + Sync + ?Sized> Send for Owned<'_, T, M> {}
// SAFETY: sharing exposes &T and requires both T and the resource to be Sync.
unsafe impl<T: Sync, M: Memory + Sync + ?Sized> Sync for Owned<'_, T, M> {}

/// A fixed-size initialized array with panic-safe partial construction.
pub struct Array<'a, T, M: Memory + ?Sized = Global> {
    block: Block<'a, M>,
    initialized: usize,
    value: PhantomData<T>,
}
impl<'a, T, M: Memory + ?Sized> Array<'a, T, M> {
    /// Construct elements in index order, dropping completed elements if `init` panics.
    pub fn new_in(count: usize, mut init: impl FnMut(usize) -> T, memory: &'a M) -> Result<Self> {
        let layout = Layout::array::<T>(count).map_err(|_| Error::InvalidLayout)?;
        let mut result = Self {
            block: Block::new_in(memory, layout, false)?,
            initialized: 0,
            value: PhantomData,
        };
        for index in 0..count {
            let value = init(index);
            // SAFETY: this slot is in bounds, properly aligned and not initialized yet.
            unsafe {
                result
                    .block
                    .as_mut_ptr()
                    .cast::<T>()
                    .add(index)
                    .write(value);
            }
            result.initialized += 1;
        }
        Ok(result)
    }
}
impl<T, M: Memory + ?Sized> Deref for Array<'_, T, M> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: precisely initialized elements are exposed, including zero-sized elements.
        unsafe { std::slice::from_raw_parts(self.block.as_ptr().cast(), self.initialized) }
    }
}
impl<T, M: Memory + ?Sized> DerefMut for Array<'_, T, M> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: &mut self exclusively borrows all initialized elements.
        unsafe { std::slice::from_raw_parts_mut(self.block.as_mut_ptr().cast(), self.initialized) }
    }
}
impl<T, M: Memory + ?Sized> Drop for Array<'_, T, M> {
    fn drop(&mut self) {
        // SAFETY: a slice drop also schedules the remaining elements if one destructor panics.
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.block.as_mut_ptr().cast::<T>(),
                self.initialized,
            ));
        }
    }
}
// SAFETY: uniquely owned initialized elements are sendable when T is Send and the resource Sync.
unsafe impl<T: Send, M: Memory + Sync + ?Sized> Send for Array<'_, T, M> {}
// SAFETY: shared access exposes only &T; both T and the resource must support sharing.
unsafe impl<T: Sync, M: Memory + Sync + ?Sized> Sync for Array<'_, T, M> {}
