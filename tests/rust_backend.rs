//! Rust mode must honor an application's existing global allocator.
#![cfg(not(any(feature = "standard", feature = "mimalloc", feature = "jemalloc")))]
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};
use unimemory::{Global, MemoryExt};

struct Counting;
static CALLS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: strictly forwards the GlobalAlloc contract to System without allocating metadata.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: caller guarantees the GlobalAlloc layout preconditions.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: caller supplies the matching System allocation and layout.
        unsafe {
            System.dealloc(pointer, layout);
        }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn selected_rust_backend_uses_the_app_allocator() {
    let global = Global::new();
    let before = CALLS.load(Ordering::Relaxed);
    let mut bytes = global.allocate_zeroed(Layout::new::<[u8; 256]>()).unwrap();
    bytes.fill(std::hint::black_box(17));
    assert_eq!(bytes.bytes().unwrap()[std::hint::black_box(3)], 17);
    assert!(CALLS.load(Ordering::Relaxed) > before);
}
