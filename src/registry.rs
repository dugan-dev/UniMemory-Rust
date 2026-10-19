//! Inline records for small groups; a reusable map for larger groups.
//! Every record retains the original native pointer's provenance.
use crate::{backend, Error, Result};
use std::{
    alloc::Layout,
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    ptr::NonNull,
};

type Record = (NonNull<u8>, Layout);

#[derive(Default)]
struct PointerHash(u64);
impl Hasher for PointerHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write_usize(&mut self, value: usize) {
        self.0 = (value as u64)
            .rotate_right(4)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.wrapping_mul(0x100_0000_01b3) ^ u64::from(byte);
        }
    }
}

pub(crate) struct Registry {
    inline: [Option<Record>; 8],
    available: u8,
    overflow: HashMap<usize, Record, BuildHasherDefault<PointerHash>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            inline: [None; 8],
            available: u8::MAX,
            overflow: HashMap::default(),
        }
    }
    #[inline]
    pub fn allocate(&mut self, layout: Layout, zeroed: bool) -> Result<NonNull<u8>> {
        if self.available != 0 {
            let slot = self.available.trailing_zeros() as usize;
            // SAFETY: memory resource provides a valid nonzero layout.
            let pointer = backend::checked(unsafe { backend::allocate(layout, zeroed) })?;
            self.inline[slot] = Some((pointer, layout));
            self.available &= !(1u8 << slot);
            return Ok(pointer);
        }
        self.overflow
            .try_reserve(1)
            .map_err(|_| Error::OutOfMemory)?;
        // SAFETY: memory resource provides a valid nonzero layout.
        let pointer = backend::checked(unsafe { backend::allocate(layout, zeroed) })?;
        self.overflow
            .insert(pointer.as_ptr().addr(), (pointer, layout));
        Ok(pointer)
    }
    #[inline]
    fn inline_slot(&self, pointer: NonNull<u8>) -> Option<usize> {
        let mut occupied = !self.available;
        while occupied != 0 {
            let slot = occupied.trailing_zeros() as usize;
            if self.inline[slot].expect("occupied record").0 == pointer {
                return Some(slot);
            }
            occupied &= occupied - 1;
        }
        None
    }
    #[inline]
    pub unsafe fn deallocate(&mut self, pointer: NonNull<u8>, layout: Layout) {
        let record = if !self.overflow.is_empty() {
            self.overflow.remove(&pointer.as_ptr().addr())
        } else {
            None
        };
        let (original, stored) = match record {
            Some(record) => record,
            None => {
                let slot = self.inline_slot(pointer).expect("matching heap allocation");
                let record = self.inline[slot].take().expect("occupied record");
                self.available |= 1u8 << slot;
                record
            }
        };
        debug_assert_eq!(stored, layout);
        // SAFETY: original retains native provenance; no payload/header reconstruction occurs.
        unsafe {
            backend::deallocate(original.as_ptr(), stored);
        }
    }
    #[inline]
    pub unsafe fn resize(
        &mut self,
        pointer: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> Result<NonNull<u8>> {
        let overflow_record = self.overflow.get(&pointer.as_ptr().addr()).copied();
        let slot = if overflow_record.is_none() {
            self.inline_slot(pointer)
        } else {
            None
        };
        let (original, stored) = overflow_record
            .or_else(|| slot.and_then(|i| self.inline[i]))
            .expect("matching heap allocation");
        debug_assert_eq!(stored, old);
        let next = if old.align() == new.align() {
            // SAFETY: exact original pointer/layout, unchanged alignment, valid new size.
            backend::checked(unsafe { backend::resize(original.as_ptr(), stored, new) })?
        } else {
            // SAFETY: valid nonzero replacement layout.
            let next = backend::checked(unsafe { backend::allocate(new, false) })?;
            // SAFETY: separate live blocks and exact original ownership, no outstanding views.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    original.as_ptr(),
                    next.as_ptr(),
                    old.size().min(new.size()),
                );
                backend::deallocate(original.as_ptr(), stored);
            }
            next
        };
        if let Some(slot) = slot {
            self.inline[slot] = Some((next, new));
        } else {
            self.overflow.remove(&pointer.as_ptr().addr());
            self.overflow.insert(next.as_ptr().addr(), (next, new));
        }
        Ok(next)
    }
    pub fn clear(&mut self) {
        for slot in &mut self.inline {
            if let Some((pointer, layout)) = slot.take() {
                // SAFETY: each record owns the exact live native allocation being released.
                unsafe {
                    backend::deallocate(pointer.as_ptr(), layout);
                }
            }
        }
        self.available = u8::MAX;
        for (_, (pointer, layout)) in self.overflow.drain() {
            // SAFETY: drained entries retain original native pointers and matching layouts.
            unsafe {
                backend::deallocate(pointer.as_ptr(), layout);
            }
        }
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.clear();
    }
}
