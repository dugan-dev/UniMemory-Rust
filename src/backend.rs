use crate::{Error, Result};
use std::{alloc::Layout, ptr::NonNull};

#[cfg(feature = "mimalloc")]
pub(crate) const NAME: &str = "mimalloc";
#[cfg(all(feature = "jemalloc", not(feature = "mimalloc")))]
pub(crate) const NAME: &str = "jemalloc";
#[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
pub(crate) const NAME: &str = "standard";

#[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
mod selected {
    use super::*;
    #[inline]
    pub unsafe fn allocate(layout: Layout, zeroed: bool) -> *mut u8 {
        // SAFETY: callers supply a nonzero valid layout for the application's global allocator.
        unsafe {
            if zeroed {
                std::alloc::alloc_zeroed(layout)
            } else {
                std::alloc::alloc(layout)
            }
        }
    }
    #[inline]
    pub unsafe fn deallocate(pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout originate from the application's global allocator.
        unsafe {
            std::alloc::dealloc(pointer, layout);
        }
    }
    #[inline]
    pub unsafe fn resize(pointer: *mut u8, old: Layout, new: Layout) -> *mut u8 {
        // SAFETY: caller guarantees a live allocation, unchanged alignment and nonzero sizes.
        unsafe { std::alloc::realloc(pointer, old, new.size()) }
    }
    pub struct NativeHeap;
    impl NativeHeap {
        pub fn new() -> Result<Self> {
            Ok(Self)
        }
        #[cfg(feature = "benchmark")]
        pub unsafe fn allocate(&self, layout: Layout, zeroed: bool) -> *mut u8 {
            // SAFETY: forwarded from resource's valid layout.
            unsafe { allocate(layout, zeroed) }
        }
        #[cfg(feature = "benchmark")]
        pub unsafe fn deallocate(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: Heap's registry guarantees this live allocation/layout pair.
            unsafe {
                deallocate(pointer, layout);
            }
        }
        pub fn collect(&self) -> Result<()> {
            Ok(())
        }
    }
}

#[cfg(feature = "mimalloc")]
mod selected {
    use super::*;
    use libmimalloc_sys as mi;
    #[inline]
    pub unsafe fn allocate(layout: Layout, zeroed: bool) -> *mut u8 {
        // SAFETY: valid size/alignment are passed unchanged to mimalloc.
        unsafe {
            if layout.align() <= std::mem::align_of::<usize>() {
                return if zeroed {
                    mi::mi_zalloc(layout.size()).cast()
                } else {
                    mi::mi_malloc(layout.size()).cast()
                };
            }
            if zeroed {
                mi::mi_zalloc_aligned(layout.size(), layout.align()).cast()
            } else {
                mi::mi_malloc_aligned(layout.size(), layout.align()).cast()
            }
        }
    }
    #[inline]
    pub unsafe fn deallocate(pointer: *mut u8, _: Layout) {
        // SAFETY: caller supplies a live mimalloc pointer.
        unsafe {
            mi::mi_free(pointer.cast());
        }
    }
    #[inline]
    pub unsafe fn resize(pointer: *mut u8, _: Layout, new: Layout) -> *mut u8 {
        // SAFETY: pointer is live; new size and alignment satisfy the original layout.
        unsafe {
            if new.align() <= std::mem::align_of::<usize>() {
                mi::mi_realloc(pointer.cast(), new.size()).cast()
            } else {
                mi::mi_realloc_aligned(pointer.cast(), new.size(), new.align()).cast()
            }
        }
    }
    pub struct NativeHeap(NonNull<mi::mi_heap_t>);
    impl NativeHeap {
        pub fn new() -> Result<Self> {
            // SAFETY: creation has no caller-owned inputs; null signals failure.
            NonNull::new(unsafe { mi::mi_heap_new() })
                .map(Self)
                .ok_or(Error::OutOfMemory)
        }
        pub unsafe fn allocate(&self, layout: Layout, zeroed: bool) -> *mut u8 {
            // SAFETY: Heap is thread-confined, owns this native heap, and layout is valid.
            unsafe {
                if layout.align() <= std::mem::align_of::<usize>() {
                    return if zeroed {
                        mi::mi_heap_zalloc(self.0.as_ptr(), layout.size()).cast()
                    } else {
                        mi::mi_heap_malloc(self.0.as_ptr(), layout.size()).cast()
                    };
                }
                if zeroed {
                    mi::mi_heap_zalloc_aligned(self.0.as_ptr(), layout.size(), layout.align())
                        .cast()
                } else {
                    mi::mi_heap_malloc_aligned(self.0.as_ptr(), layout.size(), layout.align())
                        .cast()
                }
            }
        }
        pub unsafe fn deallocate(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: pointer is a live block from this native heap.
            unsafe {
                deallocate(pointer, layout);
            }
        }
        pub unsafe fn resize(&self, pointer: *mut u8, _: Layout, new: Layout) -> *mut u8 {
            // SAFETY: caller owns this block; thread confinement and native heap identity hold.
            unsafe {
                if new.align() <= std::mem::align_of::<usize>() {
                    return mi::mi_heap_realloc(self.0.as_ptr(), pointer.cast(), new.size()).cast();
                }
                mi::mi_heap_realloc_aligned(
                    self.0.as_ptr(),
                    pointer.cast(),
                    new.size(),
                    new.align(),
                )
                .cast()
            }
        }
        pub fn reset(&mut self) -> Result<()> {
            let replacement = Self::new()?;
            *self = replacement;
            Ok(())
        }
        pub fn collect(&self) -> Result<()> {
            // SAFETY: native heap is live and accessed only on its owning thread.
            unsafe {
                mi::mi_heap_collect(self.0.as_ptr(), true);
            }
            Ok(())
        }
    }
    impl Drop for NativeHeap {
        fn drop(&mut self) {
            // SAFETY: exclusive ownership; no safe owner can outlive its borrowing resource.
            unsafe {
                mi::mi_heap_destroy(self.0.as_ptr());
            }
        }
    }
}

#[cfg(all(feature = "jemalloc", not(feature = "mimalloc")))]
mod selected {
    use super::*;
    #[cfg(target_env = "msvc")]
    use std::ffi::{c_char, c_void};
    use std::ffi::{c_int, CStr};
    #[cfg(not(target_env = "msvc"))]
    use tikv_jemalloc_sys::{calloc, dallocx, free, mallctl, malloc, mallocx, rallocx, realloc};
    #[cfg(target_env = "msvc")]
    unsafe extern "C" {
        #[link_name = "je_malloc"]
        fn malloc(size: usize) -> *mut c_void;
        #[link_name = "je_calloc"]
        fn calloc(count: usize, size: usize) -> *mut c_void;
        #[link_name = "je_free"]
        fn free(pointer: *mut c_void);
        #[link_name = "je_realloc"]
        fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void;
        #[link_name = "je_mallocx"]
        fn mallocx(size: usize, flags: c_int) -> *mut c_void;
        #[link_name = "je_rallocx"]
        fn rallocx(pointer: *mut c_void, size: usize, flags: c_int) -> *mut c_void;
        #[link_name = "je_dallocx"]
        fn dallocx(pointer: *mut c_void, flags: c_int);
        #[link_name = "je_mallctl"]
        fn mallctl(
            name: *const c_char,
            old: *mut c_void,
            old_size: *mut usize,
            new: *mut c_void,
            new_size: usize,
        ) -> c_int;
    }
    const ZERO: c_int = 0x40;
    const TCACHE_NONE: c_int = 1 << 8;
    fn flags(layout: Layout) -> c_int {
        layout.align().trailing_zeros() as c_int
    }
    #[inline]
    pub unsafe fn allocate(layout: Layout, zeroed: bool) -> *mut u8 {
        // SAFETY: nonzero size and a power-of-two alignment encode a valid mallocx request.
        unsafe {
            if layout.align() <= std::mem::align_of::<usize>() && layout.align() <= layout.size() {
                return if zeroed {
                    calloc(1, layout.size()).cast()
                } else {
                    malloc(layout.size()).cast()
                };
            }
            mallocx(layout.size(), flags(layout) | if zeroed { ZERO } else { 0 }).cast()
        }
    }
    #[inline]
    pub unsafe fn deallocate(pointer: *mut u8, _: Layout) {
        // SAFETY: a live jemalloc block is released exactly once.
        unsafe {
            free(pointer.cast());
        }
    }
    #[inline]
    pub unsafe fn resize(pointer: *mut u8, _: Layout, new: Layout) -> *mut u8 {
        // SAFETY: pointer and sizes satisfy rallocx's contract.
        unsafe {
            if new.align() <= std::mem::align_of::<usize>() && new.align() <= new.size() {
                realloc(pointer.cast(), new.size()).cast()
            } else {
                rallocx(pointer.cast(), new.size(), flags(new)).cast()
            }
        }
    }
    pub struct NativeHeap {
        arena: u32,
    }
    impl NativeHeap {
        pub fn new() -> Result<Self> {
            let mut arena = 0u32;
            let mut size = std::mem::size_of::<u32>();
            // SAFETY: output pointers identify valid unsigned storage with its exact length.
            let result = unsafe {
                mallctl(
                    c"arenas.create".as_ptr(),
                    (&mut arena as *mut u32).cast(),
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result != 0 || size != std::mem::size_of::<u32>() {
                return Err(Error::Backend("jemalloc arena creation failed"));
            }
            let heap = Self { arena };
            if arena > 2046 {
                return Err(Error::Backend("jemalloc arena exceeds mallocx flag range"));
            }
            Ok(heap)
        }
        fn arena_flags(&self) -> c_int {
            ((self.arena as c_int + 1) << 20) | TCACHE_NONE
        }
        fn command(&self, command: &str) -> Result<()> {
            let name = std::ffi::CString::new(format!("arena.{}.{}", self.arena, command))
                .expect("static arena command");
            self.control(&name)
        }
        fn control(&self, name: &CStr) -> Result<()> {
            // SAFETY: NUL-terminated control name; these commands have no input/output payload.
            if unsafe {
                mallctl(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            } != 0
            {
                return Err(Error::Backend("jemalloc arena control failed"));
            }
            Ok(())
        }
        pub unsafe fn allocate(&self, layout: Layout, zeroed: bool) -> *mut u8 {
            // SAFETY: arena is live, its ID fits the flag range, and size/alignment are valid.
            unsafe {
                mallocx(
                    layout.size(),
                    flags(layout) | self.arena_flags() | if zeroed { ZERO } else { 0 },
                )
                .cast()
            }
        }
        pub unsafe fn deallocate(&self, pointer: *mut u8, _: Layout) {
            // SAFETY: allocation belongs to this arena; its allocations bypass the thread cache.
            unsafe {
                dallocx(pointer.cast(), TCACHE_NONE);
            }
        }
        pub unsafe fn resize(&self, pointer: *mut u8, _: Layout, new: Layout) -> *mut u8 {
            // SAFETY: live block from this arena, exclusive caller ownership and valid layout.
            unsafe { rallocx(pointer.cast(), new.size(), flags(new) | self.arena_flags()).cast() }
        }
        pub fn reset(&mut self) -> Result<()> {
            self.command("reset")
        }
        pub fn collect(&self) -> Result<()> {
            self.command("purge")
        }
    }
    impl Drop for NativeHeap {
        fn drop(&mut self) {
            let _ = self.command("destroy");
        }
    }

    pub fn configure_release_delay(value: i64) -> Result<()> {
        static CONTROL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = CONTROL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut value = isize::try_from(value)
            .map_err(|_| Error::Backend("release delay exceeds native range"))?;
        fn read(name: &CStr) -> std::result::Result<isize, c_int> {
            let mut old = 0isize;
            let mut length = std::mem::size_of::<isize>();
            // SAFETY: exact writable isize output buffer and a NUL-terminated control name.
            let result = unsafe {
                mallctl(
                    name.as_ptr(),
                    (&mut old as *mut isize).cast(),
                    &mut length,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result != 0 {
                return Err(result);
            }
            if length != std::mem::size_of::<isize>() {
                return Err(-1);
            }
            Ok(old)
        }
        let mut snapshots = std::vec::Vec::new();
        for name in [c"arenas.dirty_decay_ms", c"arenas.muzzy_decay_ms"] {
            snapshots.push((
                name.to_owned(),
                read(name)
                    .map_err(|_| Error::Backend("jemalloc decay configuration unavailable"))?,
            ));
        }
        let mut epoch = 1u64;
        // SAFETY: the documented epoch write refreshes initialized-arena metadata.
        if unsafe {
            mallctl(
                c"epoch".as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                (&mut epoch as *mut u64).cast(),
                std::mem::size_of::<u64>(),
            )
        } != 0
        {
            return Err(Error::Backend("jemalloc arena metadata refresh failed"));
        }
        let mut arenas = 0u32;
        let mut length = std::mem::size_of::<u32>();
        // SAFETY: exact unsigned output buffer for the documented arenas.narenas control.
        let result = unsafe {
            mallctl(
                c"arenas.narenas".as_ptr(),
                (&mut arenas as *mut u32).cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 || length != std::mem::size_of::<u32>() {
            return Err(Error::Backend("jemalloc arena list unavailable"));
        }
        for arena in 0..arenas {
            let name = std::ffi::CString::new(format!("arena.{arena}.initialized"))
                .expect("static control name");
            let mut initialized = false;
            let mut length = std::mem::size_of::<bool>();
            // SAFETY: initialized is a documented C bool output with exactly its native size.
            let result = unsafe {
                mallctl(
                    name.as_ptr(),
                    (&mut initialized as *mut bool).cast(),
                    &mut length,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result != 0 || length != std::mem::size_of::<bool>() {
                return Err(Error::Backend("jemalloc arena state lookup failed"));
            }
            if !initialized {
                continue;
            }
            for option in ["dirty_decay_ms", "muzzy_decay_ms"] {
                let name = std::ffi::CString::new(format!("arena.{arena}.{option}"))
                    .expect("static control name");
                let old = read(&name)
                    .map_err(|_| Error::Backend("jemalloc arena decay lookup failed"))?;
                snapshots.push((name, old));
            }
        }
        for (index, (name, _)) in snapshots.iter().enumerate() {
            // SAFETY: each previously validated control accepts an isize input of this exact size.
            let result = unsafe {
                mallctl(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    (&mut value as *mut isize).cast(),
                    std::mem::size_of::<isize>(),
                )
            };
            if result != 0 {
                for (name, old) in snapshots.iter_mut().take(index + 1) {
                    // SAFETY: restore the captured value to the same validated control.
                    let _ = unsafe {
                        mallctl(
                            name.as_ptr(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            (old as *mut isize).cast(),
                            std::mem::size_of::<isize>(),
                        )
                    };
                }
                return Err(Error::Backend("jemalloc rejected release delay"));
            }
        }
        Ok(())
    }
}

#[cfg(all(feature = "jemalloc", not(feature = "mimalloc")))]
pub(crate) use selected::configure_release_delay;
pub(crate) use selected::{allocate, deallocate, resize, NativeHeap};

pub(crate) fn checked(pointer: *mut u8) -> Result<NonNull<u8>> {
    NonNull::new(pointer).ok_or(Error::OutOfMemory)
}
