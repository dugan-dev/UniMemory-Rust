//! Correctness, ownership, boundary, concurrency and deterministic stress tests.
use std::{
    alloc::Layout,
    mem::MaybeUninit,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use unimemory::{Error, Global, Heap, Memory, MemoryExt, Stack};

fn exercise<M: Memory + ?Sized>(memory: &M) {
    for alignment in [1, 2, 4, 8, 16, 64, 256, 4096] {
        for size in [0, 1, 7, 64, 257] {
            let layout = Layout::from_size_align(size, alignment).unwrap();
            let mut block = memory.allocate(layout).unwrap();
            assert_eq!(block.as_ptr().addr() % alignment, 0);
            assert_eq!(block.len(), size);
            assert_eq!(block.as_uninit_slice().len(), size);
            if size > 0 {
                assert!(block.bytes().is_none());
            }
            block.fill(0xa5);
            assert!(block.bytes().unwrap().iter().all(|&b| b == 0xa5));
        }
    }
    let value = memory.create(String::from("owned value")).unwrap();
    assert_eq!(&**value, "owned value");
    assert_eq!(value.into_inner(), "owned value");
    let array = memory.create_array(100, |i| i * i).unwrap();
    assert_eq!(array[99], 9801);
    drop(array);
    let block = memory.allocate_zeroed(Layout::new::<u64>()).unwrap();
    assert_eq!(block.bytes().unwrap(), &[0; 8]);
}

#[test]
fn global_boundaries_and_objects() {
    assert_eq!(Global::new().kind(), unimemory::MemoryKind::Global);
    exercise(&Global::new());
}

#[test]
fn heap_boundaries_and_objects() {
    let heap = Heap::new().unwrap();
    assert_eq!(heap.kind(), unimemory::MemoryKind::Heap);
    exercise(&heap);
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn stack_boundaries_and_objects() {
    let mut storage = vec![MaybeUninit::<u8>::uninit(); 256 * 1024];
    let stack = Stack::new(&mut storage);
    assert_eq!(stack.kind(), unimemory::MemoryKind::Stack);
    exercise(&stack);
    assert!(stack.used() > 0);
}

#[test]
fn dyn_memory_uses_the_actual_resource() {
    let heap = Heap::new().unwrap();
    let memory: &dyn Memory = &heap;
    let value = memory.create(42).unwrap();
    assert_eq!(*value, 42);
    assert_eq!(heap.live_allocations(), 1);
    drop(value);
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn initialized_views_are_never_fabricated() {
    let global = Global::new();
    let mut block = global
        .allocate_zeroed(Layout::from_size_align(32, 16).unwrap())
        .unwrap();
    assert_eq!(block.bytes().unwrap(), &[0; 32]);
    block.as_uninit_slice_mut()[0] = MaybeUninit::uninit();
    assert!(block.bytes().is_none());
    block.fill(9);
    assert!(block.bytes().unwrap().iter().all(|&b| b == 9));
    block.bytes_mut().unwrap()[0] = 3;
    assert_eq!(block.bytes().unwrap()[0], 3);
}

#[test]
fn raw_writers_must_restore_the_initialized_guarantee() {
    let global = Global::new();
    let mut block = global.allocate_zeroed(Layout::new::<u64>()).unwrap();
    let pointer = block.as_mut_ptr();
    assert!(block.bytes().is_none());
    // SAFETY: this pointer still owns eight writable bytes; every byte is filled.
    unsafe {
        pointer.write_bytes(17, block.len());
        block.assume_initialized();
    }
    assert_eq!(block.bytes().unwrap(), &[17; 8]);
}

#[test]
fn stack_backing_storage_may_become_uninitialized() {
    let mut buffer = [MaybeUninit::new(5u8); 64];
    {
        let stack = Stack::new(&mut buffer);
        let mut block = stack.allocate(Layout::new::<u8>()).unwrap();
        block.as_uninit_slice_mut()[0] = MaybeUninit::uninit();
        assert!(block.bytes().is_none());
    }
    buffer[0].write(17);
    // SAFETY: the preceding write initialized this exact byte.
    assert_eq!(unsafe { buffer[0].assume_init() }, 17);
}

fn resizing<M: Memory + ?Sized>(memory: &M) {
    let mut block = memory
        .allocate_zeroed(Layout::from_size_align(32, 32).unwrap())
        .unwrap();
    block.fill(3);
    block.resize_zeroed(128).unwrap();
    assert_eq!(&block.bytes().unwrap()[..32], &[3; 32]);
    assert_eq!(&block.bytes().unwrap()[32..], &[0; 96]);
    block.resize(16).unwrap();
    assert_eq!(block.bytes().unwrap(), &[3; 16]);
    block.resize(64).unwrap();
    assert!(block.bytes().is_none());
    block.fill(4);
    block.resize(0).unwrap();
    assert!(block.is_empty());
    block.resize_zeroed(23).unwrap();
    assert_eq!(block.bytes().unwrap(), &[0; 23]);
    let before = block.as_ptr();
    assert_eq!(block.resize(usize::MAX), Err(Error::InvalidLayout));
    assert_eq!(block.as_ptr(), before);
}

#[test]
fn resize_preserves_contents_for_every_resource() {
    resizing(&Global::new());
    resizing(&Heap::new().unwrap());
    let mut storage = [MaybeUninit::<u8>::uninit(); 4096];
    resizing(&Stack::new(&mut storage));
}

#[test]
fn raw_resize_can_change_alignment() {
    let global = Global::new();
    let heap = Heap::new().unwrap();
    let mut storage = [MaybeUninit::<u8>::uninit(); 8192];
    let stack = Stack::new(&mut storage);
    for resource in [&global as &dyn Memory, &heap, &stack] {
        let old = Layout::from_size_align(23, 1).unwrap();
        let new = Layout::from_size_align(128, 512).unwrap();
        let pointer = resource.allocate_raw(old, true).unwrap();
        // SAFETY: live allocation with exact old layout, no views and matching resource.
        let next = unsafe { resource.reallocate_raw(pointer, old, new, true) }.unwrap();
        assert_eq!(next.as_ptr().addr() % 512, 0);
        // SAFETY: successful zeroed growth initialized all 128 bytes.
        assert!(unsafe { std::slice::from_raw_parts(next.as_ptr(), 128) }
            .iter()
            .all(|&b| b == 0));
        // SAFETY: new layout and the successful replacement pointer match exactly.
        unsafe {
            resource.deallocate_raw(next, new);
        }
    }
}

#[test]
fn failed_stack_growth_preserves_original_block_and_cursor() {
    let mut storage = [MaybeUninit::<u8>::uninit(); 128];
    let stack = Stack::new(&mut storage);
    let mut first = stack
        .allocate_zeroed(Layout::from_size_align(40, 1).unwrap())
        .unwrap();
    first.fill(11);
    let _second = stack
        .allocate(Layout::from_size_align(40, 1).unwrap())
        .unwrap();
    let used = stack.used();
    assert_eq!(first.resize(200), Err(Error::OutOfMemory));
    assert_eq!(stack.used(), used);
    assert_eq!(first.bytes().unwrap(), &[11; 40]);
}

#[test]
fn stack_last_growth_uses_existing_storage() {
    let mut storage = [MaybeUninit::<u8>::uninit(); 256];
    let stack = Stack::new(&mut storage);
    let mut block = stack
        .allocate_zeroed(Layout::from_size_align(32, 1).unwrap())
        .unwrap();
    let pointer = block.as_ptr();
    block.resize_zeroed(64).unwrap();
    assert_eq!(block.as_ptr(), pointer);
    assert_eq!(stack.used(), 64);
    drop(block);
    assert_eq!(stack.used(), 64);
}

#[test]
fn stack_reset_rewind_and_identity() {
    let mut storage = [MaybeUninit::<u8>::uninit(); 1024];
    let old = {
        let mut stack = Stack::new(&mut storage);
        let mark = stack.mark();
        std::mem::forget(stack.allocate(Layout::new::<[u8; 64]>()).unwrap());
        stack.rewind(mark).unwrap();
        assert_eq!(stack.used(), 0);
        assert_eq!(stack.rewind(mark), Err(Error::InvalidMark));
        let mark = stack.mark();
        stack.reset().unwrap();
        assert_eq!(stack.rewind(mark), Err(Error::InvalidMark));
        stack.mark()
    };
    let mut replacement = Stack::new(&mut storage);
    assert_eq!(replacement.rewind(old), Err(Error::InvalidMark));
    let mut other_storage = [MaybeUninit::<u8>::uninit(); 1];
    let other = Stack::new(&mut other_storage);
    assert_eq!(replacement.rewind(other.mark()), Err(Error::InvalidMark));
}

#[test]
fn empty_stack_supports_only_zero_size() {
    let mut storage = [];
    let stack = Stack::new(&mut storage);
    for alignment in [1, 64, 4096] {
        let block = stack
            .allocate(Layout::from_size_align(0, alignment).unwrap())
            .unwrap();
        assert_eq!(block.as_ptr().addr() % alignment, 0);
    }
    assert_eq!(
        stack.allocate(Layout::new::<u8>()).err(),
        Some(Error::OutOfMemory)
    );
    assert_eq!(stack.used(), 0);
}

#[test]
fn heap_tracks_ownership_and_reclaims_forgotten_storage() {
    let mut first = Heap::new().unwrap();
    let second = Heap::new().unwrap();
    let block = first.allocate(Layout::new::<[u8; 64]>()).unwrap();
    assert!(first.owns(&block));
    assert!(!second.owns(&block));
    std::mem::forget(block);
    assert_eq!(first.live_allocations(), 1);
    first.collect().unwrap();
    first.reset().unwrap();
    assert_eq!(first.live_allocations(), 0);
    assert_eq!(*first.create(17).unwrap(), 17);
}

struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn owned_and_array_run_destructors_exactly_once() {
    let heap = Heap::new().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let value = heap.create(DropCount(count.clone())).unwrap().into_inner();
    assert_eq!(heap.live_allocations(), 0);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    drop(value);
    let array = heap.create_array(9, |_| DropCount(count.clone())).unwrap();
    drop(array);
    assert_eq!(count.load(Ordering::SeqCst), 10);
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn array_constructor_panic_rolls_back_and_releases_storage() {
    let heap = Heap::new().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let result = catch_unwind(AssertUnwindSafe(|| {
        heap.create_array(10, |i| {
            if i == 4 {
                panic!("synthetic constructor failure");
            }
            DropCount(count.clone())
        })
    }));
    assert!(result.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 4);
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn destructor_panic_still_releases_storage() {
    struct PanicDrop;
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            panic!("synthetic destructor failure");
        }
    }
    let heap = Heap::new().unwrap();
    let value = heap.create((42u64, PanicDrop)).unwrap();
    assert_eq!(heap.live_allocations(), 1);
    assert!(catch_unwind(AssertUnwindSafe(|| drop(value))).is_err());
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn zero_sized_values_and_array_overflow() {
    let global = Global::new();
    assert_eq!(*global.create(()).unwrap(), ());
    let empty = global.create_array::<u64>(0, |_| unreachable!()).unwrap();
    assert!(empty.is_empty());
    let result = global.create_array::<u64>(usize::MAX, |_| unreachable!());
    assert_eq!(result.err(), Some(Error::InvalidLayout));
    #[repr(align(4096))]
    struct Aligned;
    let aligned = global.create(Aligned).unwrap();
    assert_eq!((&*aligned as *const Aligned).addr() % 4096, 0);
}

#[test]
fn global_is_shared_and_owners_can_move_between_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Global>();
    let memory = Global::new();
    std::thread::scope(|scope| {
        let value = memory.create(String::from("transfer")).unwrap();
        let block = memory.allocate_zeroed(Layout::new::<[u8; 32]>()).unwrap();
        scope.spawn(move || {
            assert_eq!(&**value, "transfer");
            assert_eq!(block.bytes().unwrap(), &[0; 32]);
        });
        for worker in 0..8 {
            let memory = &memory;
            scope.spawn(move || {
                for index in 0..if cfg!(miri) { 20 } else { 2000 } {
                    let value = memory.create((worker, index)).unwrap();
                    assert_eq!(*value, (worker, index));
                }
            });
        }
    });
}

#[test]
fn deterministic_mixed_allocation_trace() {
    let global = Global::new();
    let heap = Heap::new().unwrap();
    for resource in [&global as &dyn Memory, &heap] {
        let mut state = 0x1234_5678u64;
        let mut blocks = std::vec::Vec::new();
        for round in 0..if cfg!(miri) { 80 } else { 5000 } {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if !blocks.is_empty() && state % 3 == 0 {
                let index = state as usize % blocks.len();
                let block: unimemory::Block<'_, dyn Memory> = blocks.swap_remove(index);
                assert!(block.bytes().unwrap().iter().all(|&b| b == 0x5a));
            } else {
                let size = state as usize % 2048 + 1;
                let alignment = 1 << (round % 9);
                let mut block = resource
                    .allocate(Layout::from_size_align(size, alignment).unwrap())
                    .unwrap();
                block.fill(0x5a);
                blocks.push(block);
            }
        }
        drop(blocks);
    }
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn zero_sized_array_elements_still_run_all_destructors() {
    static DROPPED: AtomicUsize = AtomicUsize::new(0);
    struct Element;
    impl Drop for Element {
        fn drop(&mut self) {
            DROPPED.fetch_add(1, Ordering::SeqCst);
        }
    }
    let memory = Global::new();
    let array = memory.create_array(17, |_| Element).unwrap();
    assert_eq!(array.len(), 17);
    drop(array);
    assert_eq!(DROPPED.load(Ordering::SeqCst), 17);
}

#[test]
fn typed_overalignment_is_preserved() {
    #[repr(align(512))]
    struct Aligned(u64);
    let global = Global::new();
    let heap = Heap::new().unwrap();
    let mut buffer = [MaybeUninit::<u8>::uninit(); 8192];
    let stack = Stack::new(&mut buffer);
    for resource in [&global as &dyn Memory, &heap, &stack] {
        let array = resource
            .create_array(3, |index| Aligned(index as u64))
            .unwrap();
        assert_eq!(array.as_ptr().addr() % 512, 0);
        assert_eq!(array[2].0, 2);
    }
}

#[test]
fn heap_inline_and_overflow_records_survive_resize_and_reuse() {
    let heap = Heap::new().unwrap();
    let mut records = std::vec::Vec::new();
    for index in 0..24 {
        let old = Layout::from_size_align(index + 8, 8).unwrap();
        let pointer = heap.allocate_raw(old, false).unwrap();
        // SAFETY: unique live allocation of exactly old.size() bytes.
        unsafe {
            pointer.as_ptr().write_bytes(index as u8, old.size());
        }
        records.push((pointer, old, index as u8));
    }
    for (index, record) in records.iter_mut().enumerate() {
        let new =
            Layout::from_size_align(128 + index, if index % 2 == 0 { 64 } else { 8 }).unwrap();
        // SAFETY: exact live original pointer/layout, no references remain.
        let pointer = unsafe { heap.reallocate_raw(record.0, record.1, new, true) }.unwrap();
        // SAFETY: the old bytes were initialized and the newly grown range was zeroed.
        let bytes = unsafe { std::slice::from_raw_parts(pointer.as_ptr(), new.size()) };
        assert!(bytes[..record.1.size()]
            .iter()
            .all(|&byte| byte == record.2));
        assert!(bytes[record.1.size()..].iter().all(|&byte| byte == 0));
        *record = (pointer, new, record.2);
    }
    while !records.is_empty() {
        let index = (records.len() * 7 / 11) % records.len();
        let (pointer, layout, _) = records.swap_remove(index);
        // SAFETY: this is the successful replacement pointer with its exact current layout.
        unsafe {
            heap.deallocate_raw(pointer, layout);
        }
        let temporary = heap
            .allocate(Layout::from_size_align(16, 64).unwrap())
            .unwrap();
        assert!(heap.owns(&temporary));
        drop(temporary);
        assert_eq!(heap.live_allocations(), records.len());
    }
    assert_eq!(heap.live_allocations(), 0);
}
