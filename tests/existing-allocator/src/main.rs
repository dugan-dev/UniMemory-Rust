use unimemory::{Global, Heap, Memory, MemoryExt, StatisticsMode};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> unimemory::Result<()> {
    assert_eq!(unimemory::BACKEND, "standard");
    let resource = Global::with_statistics(StatisticsMode::Basic)?;
    {
        let value = resource.create(String::from("existing allocator"))?;
        assert_eq!(&**value, "existing allocator");
    }
    assert_eq!(resource.statistics().live_bytes, 0);
    let mut heap = Heap::new()?;
    {
        let array = heap.create_array(128, |i| i * i)?;
        assert_eq!(array[127], 16129);
    }
    heap.reset()?;
    println!("Standard backend reused the existing mimalloc global allocator");
    Ok(())
}
