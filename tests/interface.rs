//! Exercises every public API item on all three resources.

use std::alloc::Layout;
use std::mem::MaybeUninit;
#[cfg(feature = "statistics")]
use unimemory::Statistics;
#[cfg(feature = "containers")]
use unimemory::Vec;
use unimemory::{
    Array, Block, Error, Global, Heap, Mark, Memory, MemoryExt, MemoryKind, Owned, Stack,
    StatisticsMode, BACKEND,
};

fn layout(bytes: usize, align: usize) -> Layout {
    Layout::from_size_align(bytes, align).expect("valid layout")
}

#[test]
fn backend_name_is_one_of_three() {
    assert!(["standard", "mimalloc", "jemalloc"].contains(&BACKEND));
}

#[test]
fn global_raw_and_typed_api() {
    let global = Global::new();
    assert_eq!(global.kind(), MemoryKind::Global);
    assert_eq!(global.statistics_mode(), StatisticsMode::Disabled);

    let raw = global.allocate_raw(layout(128, 64), false).unwrap();
    // SAFETY: freshly allocated, uniquely owned block of 128 bytes.
    unsafe { raw.as_ptr().write_bytes(7, 128) };
    let grown = layout(256, 64);
    // SAFETY: live allocation with the exact original layout.
    let grown_ptr = unsafe { global.reallocate_raw(raw, layout(128, 64), grown, false) }.unwrap();
    // SAFETY: reallocation preserved the prefix.
    assert_eq!(unsafe { *grown_ptr.as_ptr() }, 7);
    // SAFETY: matching allocation and layout.
    unsafe { global.deallocate_raw(grown_ptr, grown) };

    let mut block: Block<'_, Global> = global.allocate(layout(128, 64)).unwrap();
    assert_eq!(block.len(), 128);
    assert_eq!(block.layout(), layout(128, 64));
    assert!(block.bytes().is_none());
    block.fill(1);
    assert_eq!(block.bytes().unwrap()[127], 1);
    assert_eq!(block.bytes_mut().unwrap().len(), 128);
    block.resize_zeroed(256).unwrap();
    assert_eq!(block.len(), 256);
    assert_eq!(block.bytes().unwrap()[255], 0);
    block.resize(0).unwrap();
    assert!(block.is_empty());
    assert_eq!(block.resource().kind(), MemoryKind::Global);

    let zeroed = global.allocate_zeroed(layout(32, 8)).unwrap();
    assert_eq!(zeroed.bytes().unwrap(), &[0u8; 32]);

    let mut raw_view = global.allocate(layout(64, 16)).unwrap();
    assert_eq!(raw_view.as_uninit_slice().len(), 64);
    // SAFETY: the unique owner initializes every byte before claiming them.
    unsafe {
        raw_view.as_mut_ptr().write_bytes(3, 64);
        raw_view.assume_initialized();
    }
    assert_eq!(raw_view.bytes().unwrap()[0], 3);
    assert_eq!(raw_view.as_uninit_slice_mut().len(), 64);

    let owned: Owned<'_, i32, Global> = global.create(41).unwrap();
    assert_eq!(*owned, 41);
    assert_eq!(owned.into_inner(), 41);

    let mut array: Array<'_, u16, Global> = global.create_array(4, |i| i as u16).unwrap();
    array[1] = 9;
    assert_eq!(&array[..], &[0, 9, 2, 3]);
}

#[test]
fn heap_api() {
    let mut heap = Heap::new().unwrap();
    assert_eq!(heap.kind(), MemoryKind::Heap);
    assert_eq!(heap.live_allocations(), 0);

    let block = heap.allocate(layout(64, 16)).unwrap();
    assert!(heap.owns(&block));
    assert_eq!(heap.live_allocations(), 1);
    drop(block);
    assert_eq!(heap.live_allocations(), 0);

    heap.collect().unwrap();
    {
        let _value = heap.create(1u8).unwrap();
    }
    heap.reset().unwrap();
    assert_eq!(heap.live_allocations(), 0);
}

#[test]
fn stack_api() {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 512];
    let mut stack = Stack::new(&mut buffer);
    assert_eq!(stack.capacity(), 512);
    assert_eq!(stack.used(), 0);

    let mark: Mark = stack.mark();
    let block = stack.allocate(layout(64, 8)).unwrap();
    assert!(stack.used() >= 64);
    drop(block);
    stack.rewind(mark).unwrap();
    assert_eq!(stack.used(), 0);
    assert!(matches!(stack.rewind(mark), Err(Error::InvalidMark)));

    stack.reset().unwrap();
    assert!(matches!(
        stack.allocate(layout(1024, 8)),
        Err(Error::OutOfMemory)
    ));
}

#[test]
fn errors_render() {
    for error in [
        Error::OutOfMemory,
        Error::InvalidLayout,
        Error::StatisticsDisabled,
        Error::Busy,
        Error::InvalidMark,
        Error::Backend("native"),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[cfg(feature = "statistics")]
#[test]
fn statistics_control_api() {
    let mut heap = Heap::with_statistics(StatisticsMode::Basic).unwrap();
    assert_eq!(heap.statistics_mode(), StatisticsMode::Basic);
    {
        let value = heap.create([0u8; 8]).unwrap();
        drop(value);
    }
    let stats: Statistics = heap.statistics();
    assert!(stats.allocations >= 1);
    heap.reset_statistics().unwrap();
    heap.set_statistics(StatisticsMode::Disabled).unwrap();
    assert_eq!(heap.statistics(), Statistics::default());

    let mut stack_buffer = [MaybeUninit::<u8>::uninit(); 128];
    let mut stack = Stack::with_statistics(&mut stack_buffer, StatisticsMode::Basic).unwrap();
    assert_eq!(stack.statistics_mode(), StatisticsMode::Basic);
    stack.set_statistics(StatisticsMode::Disabled).unwrap();
    stack.reset_statistics().unwrap();
}

#[cfg(feature = "containers")]
#[test]
fn containers_and_adapter_api() {
    use allocator_api2::alloc::Allocator;

    let global = Global::new();
    let mut values = Vec::new_in(global.allocator());
    values.extend(0u64..128);
    assert_eq!(values.iter().sum::<u64>(), 8128);

    let boxed = unimemory::Box::new_in(5u32, global.allocator());
    assert_eq!(*boxed, 5);

    let adapter = global.allocator();
    let small = layout(16, 8);
    let large = layout(32, 8);
    let pointer = adapter.allocate(small).unwrap();
    // SAFETY: live allocation from this adapter with the exact old layout.
    let grown = unsafe { adapter.grow(pointer.cast(), small, large) }.unwrap();
    // SAFETY: matching allocation and layout.
    unsafe { adapter.deallocate(grown.cast(), large) };
}

#[test]
fn native_allocator_global_alloc_contract() {
    use std::alloc::GlobalAlloc;

    let allocator = unimemory::NativeAllocator;
    let small = layout(64, 32);
    // SAFETY: valid nonzero layout, matching deallocation below.
    unsafe {
        let pointer = allocator.alloc(small);
        assert!(!pointer.is_null());
        pointer.write_bytes(9, 64);
        allocator.dealloc(pointer, small);

        let zeroed = allocator.alloc_zeroed(small);
        assert!(!zeroed.is_null());
        assert_eq!(*zeroed, 0);
        let larger = layout(128, 32);
        let resized = allocator.realloc(zeroed, small, larger.size());
        assert!(!resized.is_null());
        allocator.dealloc(resized, larger);
    }
}

#[test]
fn dyn_memory_is_object_safe() {
    let heap = Heap::new().unwrap();
    let resource: &dyn Memory = &heap;
    assert_eq!(resource.kind(), MemoryKind::Heap);
    let block = resource.allocate(layout(8, 8)).unwrap();
    drop(block);
}
