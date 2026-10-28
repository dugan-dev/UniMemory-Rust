//! Scenario tests that mirror real usage: frame arenas, scratch parsing,
//! request-scoped heaps and mixed-lifetime resources.

use std::alloc::Layout;
use std::mem::MaybeUninit;
use unimemory::{Global, Heap, Memory, MemoryExt, Stack};

fn layout(bytes: usize, align: usize) -> Layout {
    Layout::from_size_align(bytes, align).expect("valid layout")
}

#[test]
fn frame_arena_reuses_one_heap() {
    let mut frame = Heap::new().unwrap();
    for tick in 0..200u64 {
        let mut owners = std::vec::Vec::new();
        for entity in 0..50u64 {
            owners.push(frame.create((tick, entity)).unwrap());
        }
        for (entity, owner) in owners.iter().enumerate() {
            assert_eq!(owner.0, tick);
            assert_eq!(owner.1, entity as u64);
        }
        drop(owners);
        frame.reset().unwrap();
        assert_eq!(frame.live_allocations(), 0);
    }
}

#[test]
fn scratch_parser_with_nested_checkpoints() {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    let mut scratch = Stack::new(&mut buffer);

    let outer = scratch.mark();
    {
        let _token = scratch.allocate(layout(128, 8)).unwrap();
    }
    scratch.rewind(outer).unwrap();

    let inner = scratch.mark();
    {
        let _nested = scratch.allocate(layout(256, 16)).unwrap();
    }
    scratch.rewind(inner).unwrap();

    {
        let _retry = scratch.allocate(layout(64, 8)).unwrap();
    }
    assert_eq!(scratch.used(), 64);
    scratch.reset().unwrap();
    assert_eq!(scratch.used(), 0);
}

#[cfg(feature = "containers")]
#[test]
fn request_scoped_heap_with_containers() {
    let mut request = Heap::new().unwrap();
    for request_id in 0..100u64 {
        let mut fields = unimemory::Vec::new_in(request.allocator());
        fields.extend((0..64).map(|index| index + request_id));
        assert_eq!(fields.len(), 64);
        assert_eq!(fields[63], 63 + request_id);
        drop(fields);

        let payload = request
            .create_array(32, |index| index as u64 * request_id)
            .unwrap();
        assert_eq!(payload[31], 31 * request_id);
        drop(payload);

        request.reset().unwrap();
        assert_eq!(request.live_allocations(), 0);
    }
}

struct Node {
    id: u64,
    links: std::vec::Vec<u64>,
}

#[test]
fn object_pool_lifecycle() {
    let mut pool = Heap::new().unwrap();
    let mut nodes = std::vec::Vec::new();
    for id in 0..300u64 {
        let node = pool
            .create(Node {
                id,
                links: (0..8).map(|offset| id + offset).collect(),
            })
            .unwrap();
        assert_eq!(node.id, id);
        assert_eq!(node.links[7], id + 7);
        nodes.push(node);
    }
    while nodes.len() > 100 {
        nodes.pop();
    }
    assert_eq!(nodes.len(), 100);
    drop(nodes);
    pool.reset().unwrap();
    assert_eq!(pool.live_allocations(), 0);
}

#[test]
fn mixed_lifetimes_share_no_state() {
    let global = Global::new();
    let mut per_request = Heap::new().unwrap();
    let mut buffer = [MaybeUninit::<u8>::uninit(); 512];
    let mut scratch = Stack::new(&mut buffer);

    let shared = global.create(String::from("shared")).unwrap();
    let checkpoint = scratch.mark();
    {
        let _temporary = scratch.create_array(16, |index| index as u8).unwrap();
    }

    {
        let owned = per_request.create(vec![1u8, 2, 3]).unwrap();
        assert_eq!(&*owned, &[1, 2, 3]);
    }
    per_request.reset().unwrap();
    scratch.rewind(checkpoint).unwrap();

    assert_eq!(&*shared, "shared");
    assert_eq!(per_request.live_allocations(), 0);
    assert_eq!(scratch.used(), 0);
}

#[cfg(feature = "statistics")]
#[test]
fn statistics_follow_scenario_traffic() {
    let mut frame = Heap::with_statistics(unimemory::StatisticsMode::Basic).unwrap();
    for _ in 0..50 {
        let owners: std::vec::Vec<_> = (0..20u8)
            .map(|value| frame.create([value; 4]).unwrap())
            .collect();
        let statistics = frame.statistics();
        assert!(statistics.live_bytes > 0);
        assert!(statistics.peak_live_bytes >= statistics.live_bytes);
        drop(owners);
        frame.reset().unwrap();
        assert_eq!(frame.statistics().live_bytes, 0);
    }
}
