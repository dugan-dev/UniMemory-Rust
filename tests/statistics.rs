//! Statistics and mode-change regressions.
use std::alloc::Layout;
use std::mem::MaybeUninit;
#[cfg(feature = "statistics")]
use std::sync::Arc;
use unimemory::{Global, Heap, Memory, MemoryExt, Stack, Statistics, StatisticsMode};

#[test]
fn disabled_is_zero_for_all_resources() {
    let global = Global::new();
    let heap = Heap::new().unwrap();
    let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
    let stack = Stack::new(&mut buffer);
    for memory in [&global as &dyn Memory, &heap, &stack] {
        let block = memory.allocate(Layout::new::<u64>()).unwrap();
        assert_eq!(memory.statistics(), Statistics::default());
        assert_eq!(memory.statistics_mode(), StatisticsMode::Disabled);
        drop(block);
    }
}

#[cfg(not(feature = "statistics"))]
#[test]
fn basic_requires_compiled_support() {
    assert!(matches!(
        Global::with_statistics(StatisticsMode::Basic),
        Err(unimemory::Error::StatisticsDisabled)
    ));
    assert!(matches!(
        Heap::with_statistics(StatisticsMode::Basic),
        Err(unimemory::Error::StatisticsDisabled)
    ));
}

#[cfg(feature = "statistics")]
fn counters<M: Memory + ?Sized>(memory: &M) {
    let mut block = memory
        .allocate(Layout::from_size_align(32, 16).unwrap())
        .unwrap();
    assert_eq!(
        memory.statistics(),
        Statistics {
            allocations: 1,
            live_bytes: 32,
            peak_live_bytes: 32,
            ..Statistics::default()
        }
    );
    block.resize_zeroed(128).unwrap();
    assert_eq!(memory.statistics().reallocations, 1);
    assert_eq!(memory.statistics().live_bytes, 128);
    block.resize(16).unwrap();
    assert_eq!(memory.statistics().peak_live_bytes, 128);
    drop(block);
    let stats = memory.statistics();
    assert_eq!(stats.allocations, 1);
    assert_eq!(stats.deallocations, 1);
    assert_eq!(stats.reallocations, 2);
    assert_eq!(stats.live_bytes, 0);
}

#[cfg(feature = "statistics")]
#[test]
fn requested_byte_counters_are_consistent_for_all_resources() {
    counters(&Global::with_statistics(StatisticsMode::Basic).unwrap());
    counters(&Heap::with_statistics(StatisticsMode::Basic).unwrap());
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    counters(&Stack::with_statistics(&mut buffer, StatisticsMode::Basic).unwrap());
}

#[cfg(feature = "statistics")]
#[test]
fn modes_pause_and_reset_while_idle() {
    let mut memory = Global::new();
    drop(memory.create(7).unwrap());
    memory.set_statistics(StatisticsMode::Basic).unwrap();
    drop(memory.create(1u64).unwrap());
    assert_eq!(memory.statistics().allocations, 1);
    memory.set_statistics(StatisticsMode::Disabled).unwrap();
    drop(memory.create(2u64).unwrap());
    assert_eq!(memory.statistics(), Statistics::default());
    memory.set_statistics(StatisticsMode::Basic).unwrap();
    assert_eq!(memory.statistics().allocations, 1);
    memory.reset_statistics().unwrap();
    assert_eq!(memory.statistics(), Statistics::default());
    let raw = memory.allocate_raw(Layout::new::<u64>(), false).unwrap();
    assert_eq!(
        memory.set_statistics(StatisticsMode::Disabled),
        Err(unimemory::Error::Busy)
    );
    assert_eq!(memory.reset_statistics(), Err(unimemory::Error::Busy));
    // SAFETY: the exact original raw allocation remains live, with no references or owners.
    unsafe {
        memory.deallocate_raw(raw, Layout::new::<u64>());
    }
}

#[cfg(feature = "statistics")]
#[test]
fn rewind_accounts_only_for_discarded_allocations() {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    let mut memory = Stack::with_statistics(&mut buffer, StatisticsMode::Basic).unwrap();
    let old_layout = Layout::new::<u64>();
    let old = memory.allocate_raw(old_layout, false).unwrap();
    let mark = memory.mark();
    std::mem::forget(memory.create_array(32, |i| i as u64).unwrap());
    assert_eq!(memory.statistics().live_bytes, 264);
    memory.rewind(mark).unwrap();
    assert_eq!(memory.statistics().live_bytes, 8);
    // SAFETY: this allocation precedes the mark and rewind preserved it; no references exist.
    unsafe {
        memory.deallocate_raw(old, old_layout);
    }
    assert_eq!(memory.statistics().live_bytes, 0);
    memory.reset().unwrap();
    assert_eq!(memory.statistics(), Statistics::default());
}

#[cfg(feature = "statistics")]
#[test]
fn reset_reclaims_forgotten_heap_allocations_and_counters() {
    let mut heap = Heap::with_statistics(StatisticsMode::Basic).unwrap();
    std::mem::forget(heap.create(99u64).unwrap());
    assert_eq!(heap.statistics().live_bytes, 8);
    heap.reset().unwrap();
    assert_eq!(heap.statistics(), Statistics::default());
    assert_eq!(heap.statistics_mode(), StatisticsMode::Basic);
}

#[cfg(feature = "statistics")]
#[test]
fn rewind_invalidates_a_block_grown_past_the_checkpoint() {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
    let mut stack = Stack::with_statistics(&mut buffer, StatisticsMode::Basic).unwrap();
    let old = Layout::from_size_align(8, 1).unwrap();
    let pointer = stack.allocate_raw(old, false).unwrap();
    let mark = stack.mark();
    let new = Layout::from_size_align(64, 1).unwrap();
    // SAFETY: exact original pointer and layout, no references or safe owners exist.
    let _grown = unsafe { stack.reallocate_raw(pointer, old, new, false) }.unwrap();
    assert_eq!(stack.statistics().live_bytes, 64);
    stack.rewind(mark).unwrap();
    assert_eq!(stack.statistics().live_bytes, 0);
    assert_eq!(stack.used(), 8);
}

#[cfg(feature = "statistics")]
#[test]
fn concurrent_statistics_and_cross_thread_free_balance() {
    let global = Arc::new(Global::with_statistics(StatisticsMode::Basic).unwrap());
    let iterations = if cfg!(miri) { 20 } else { 1000 };
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let global = &global;
            scope.spawn(move || {
                for index in 0..iterations {
                    drop(global.create((worker as u64, index)).unwrap());
                }
            });
        }
        let block = global.allocate_zeroed(Layout::new::<[u8; 64]>()).unwrap();
        scope.spawn(move || drop(block));
    });
    let stats = global.statistics();
    assert_eq!(stats.allocations, 8 * iterations + 1);
    assert_eq!(stats.deallocations, 8 * iterations + 1);
    assert_eq!(stats.live_bytes, 0);
    assert!(stats.peak_live_bytes >= 64);
}
