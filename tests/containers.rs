//! Stable allocator-aware container integration.
#![cfg(feature = "containers")]
use allocator_api2::alloc::Allocator;
use std::mem::MaybeUninit;
use std::{
    alloc::Layout,
    panic::{catch_unwind, AssertUnwindSafe},
};
use unimemory::{Global, Heap, Memory, MemoryExt, Stack, Vec};

fn roundtrip<M: Memory + ?Sized>(memory: &M) {
    let mut values = Vec::new_in(memory.allocator());
    for i in 0..4096 {
        values.push(i as u64);
    }
    assert_eq!(values.iter().sum::<u64>(), 8_386_560);
    values.truncate(17);
    values.shrink_to_fit();
    assert_eq!(values.capacity(), 17);
    values.extend(17..1024);
    assert_eq!(values[1023], 1023);
    let text = unimemory::Box::new_in(String::from("allocator-aware"), memory.allocator());
    assert_eq!(&**text, "allocator-aware");
    let mut zeros = Vec::new_in(memory.allocator());
    zeros.extend(std::iter::repeat_n((), 128));
    assert_eq!(zeros.len(), 128);
}

#[test]
fn vectors_and_boxes_use_each_resource() {
    roundtrip(&Global::new());
    let heap = Heap::new().unwrap();
    roundtrip(&heap);
    assert_eq!(heap.live_allocations(), 0);
    let mut storage = vec![MaybeUninit::<u8>::uninit(); 256 * 1024];
    roundtrip(&Stack::new(&mut storage));
}

#[test]
fn try_reserve_reports_exhaustion_without_losing_elements() {
    let mut storage = [MaybeUninit::<u8>::uninit(); 128];
    let stack = Stack::new(&mut storage);
    let mut values = Vec::new_in(stack.allocator());
    values.push(42u64);
    assert!(values.try_reserve(1024).is_err());
    assert_eq!(&values[..], &[42]);
}

#[test]
fn adapter_zero_size_and_changing_alignment() {
    let memory = Global::new();
    let adapter = memory.allocator();
    let old = Layout::from_size_align(0, 8).unwrap();
    let pointer = adapter.allocate(old).unwrap().cast::<u8>();
    let grown = Layout::from_size_align(128, 256).unwrap();
    // SAFETY: exact matching zero-sized allocation and valid larger layout.
    let pointer = unsafe { adapter.grow_zeroed(pointer, old, grown) }
        .unwrap()
        .cast::<u8>();
    // SAFETY: grow_zeroed initialized the whole new block.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.as_ptr(), 128) };
    assert_eq!(bytes, &[0; 128]);
    let small = Layout::from_size_align(0, 512).unwrap();
    // SAFETY: no references remain, pointer/layout match and new layout has smaller size.
    let pointer = unsafe { adapter.shrink(pointer, grown, small) }
        .unwrap()
        .cast::<u8>();
    assert_eq!(pointer.as_ptr().addr() % 512, 0);
    // SAFETY: matching zero-sized allocation is accepted by the Allocator contract.
    unsafe {
        adapter.deallocate(pointer, small);
    }
}

#[test]
fn panicking_vector_element_destructor_still_releases_storage() {
    struct Item(bool);
    impl Drop for Item {
        fn drop(&mut self) {
            assert!(!self.0, "synthetic element failure");
        }
    }
    let heap = Heap::new().unwrap();
    let mut values = Vec::new_in(heap.allocator());
    values.extend([Item(false), Item(true), Item(false)]);
    assert!(catch_unwind(AssertUnwindSafe(|| drop(values))).is_err());
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn global_allocator_aware_vector_can_cross_threads() {
    let global = Global::new();
    std::thread::scope(|scope| {
        let mut values = Vec::new_in(global.allocator());
        values.extend(0..1024);
        scope.spawn(move || assert_eq!(values.iter().sum::<u64>(), 523_776));
    });
}
