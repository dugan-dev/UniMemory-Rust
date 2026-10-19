//! Process-wide configuration and release policy controls.
#[test]
fn release_delay_validation_and_supported_controls() {
    use unimemory::MemoryExt;
    let heap = unimemory::Heap::new().unwrap();
    let value = heap.create([17u64; 8]).unwrap();
    assert!(unimemory::config(-2).is_err());
    unimemory::config(-1).unwrap();
    unimemory::config(0).unwrap();
    unimemory::Global::config(100).unwrap();
    assert_eq!(*value, [17; 8]);
    #[cfg(feature = "mimalloc")]
    assert!(unimemory::config(i64::MAX).is_err());
}
