//! Allocator-aware containers on stable Rust.
#[cfg(feature = "containers")]
fn main() -> unimemory::Result<()> {
    use unimemory::{Heap, MemoryExt, Vec};
    let mut heap = Heap::new()?;
    {
        let mut values = Vec::new_in(heap.allocator());
        values.extend(0..1024);
        assert_eq!(values.iter().sum::<u32>(), 523_776);
        let value = unimemory::Box::new_in(42, heap.allocator());
        assert_eq!(*value, 42);
    }
    heap.reset()?;
    Ok(())
}

#[cfg(not(feature = "containers"))]
fn main() {}
