//! Stress and boundary coverage across all three resources.

use std::alloc::Layout;
use std::mem::MaybeUninit;
use std::sync::Arc;
use unimemory::{Global, Heap, Memory, MemoryExt, Stack};

fn layout(bytes: usize, align: usize) -> Layout {
    Layout::from_size_align(bytes, align).expect("valid layout")
}

#[test]
fn global_mixed_size_churn() {
    let global = Global::new();
    let mut live = std::vec::Vec::new();
    for round in 0..4_000usize {
        let bytes = 8 + (round * 37) % 4096;
        let align = 1 << (round % 7);
        let pointer = global.allocate_raw(layout(bytes, align), false).unwrap();
        // SAFETY: unique live allocation of at least `bytes` bytes.
        unsafe { pointer.as_ptr().write_bytes((round % 251) as u8, bytes) };
        live.push((pointer, layout(bytes, align), (round % 251) as u8));
        if live.len() > 64 {
            let (pointer, layout, value) = live.remove(0);
            // SAFETY: exact pointer/layout pair from this resource.
            unsafe {
                assert_eq!(*pointer.as_ptr(), value);
                global.deallocate_raw(pointer, layout);
            }
        }
    }
    for (pointer, layout, _) in live {
        // SAFETY: exact pointer/layout pair from this resource.
        unsafe { global.deallocate_raw(pointer, layout) };
    }
}

#[test]
fn alignment_matrix() {
    let global = Global::new();
    for power in 0..=12u32 {
        let align = 1usize << power;
        let bytes = align.max(8) * 2;
        let block = global.allocate(layout(bytes, align)).unwrap();
        assert_eq!(block.as_ptr() as usize % align, 0);
        // SAFETY: the block owns `bytes` writable bytes.
        unsafe { block.as_ptr().cast_mut().write_bytes(0xAB, bytes) };
    }
}

#[test]
fn heap_reset_cycles_do_not_leak() {
    let mut heap = Heap::new().unwrap();
    for _ in 0..100 {
        let mut owners = std::vec::Vec::new();
        for index in 0..100u64 {
            owners.push(heap.create(index).unwrap());
        }
        assert_eq!(heap.live_allocations(), 100);
        for (expected, owner) in owners.iter().enumerate() {
            assert_eq!(**owner, expected as u64);
        }
        drop(owners);
        assert_eq!(heap.live_allocations(), 0);
        heap.reset().unwrap();
    }
}

#[test]
fn stack_mark_rewind_churn() {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
    let mut stack = Stack::new(&mut buffer);
    for round in 0..5_000usize {
        let mark = stack.mark();
        let block = stack.allocate(layout(8 + round % 64, 8)).unwrap();
        assert!(stack.used() >= 8);
        drop(block);
        stack.rewind(mark).unwrap();
        assert_eq!(stack.used(), 0);
    }
}

#[test]
fn block_resize_churn_preserves_prefix() {
    let global = Global::new();
    let mut block = global.allocate(layout(16, 8)).unwrap();
    block.fill(0x5A);
    for round in 0..1_000usize {
        let target = 16 + (round * 61) % 8192;
        block.resize(target).unwrap();
        assert_eq!(block.len(), target);
        // SAFETY: fill initialized the prefix and resize preserves it.
        unsafe { block.assume_initialized() };
        let bytes = block.bytes().unwrap();
        assert!(bytes.iter().take(16).all(|byte| *byte == 0x5A));
    }
}

#[test]
fn global_is_shared_between_threads() {
    let global = Arc::new(Global::new());
    let mut handles = std::vec::Vec::new();
    for thread in 0..4u8 {
        let global = Arc::clone(&global);
        handles.push(std::thread::spawn(move || {
            for round in 0..2_000usize {
                let bytes = 16 + (round * 13) % 512;
                let block = global.allocate(layout(bytes, 8)).unwrap();
                // SAFETY: unique block of `bytes` writable bytes.
                unsafe { block.as_ptr().cast_mut().write_bytes(thread, bytes) };
                assert_eq!(block.as_ptr() as usize % 8, 0);
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
}

#[cfg(feature = "statistics")]
#[test]
fn statistics_stay_balanced_under_churn() {
    let global = Global::with_statistics(unimemory::StatisticsMode::Basic).unwrap();
    for round in 0..2_000usize {
        let block = global.allocate(layout(32 + round % 128, 8)).unwrap();
        drop(block);
    }
    let statistics = global.statistics();
    assert_eq!(statistics.live_bytes, 0);
    assert!(statistics.allocations >= 2_000);
    assert_eq!(statistics.allocations, statistics.deallocations);
}
