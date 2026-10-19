//! Basic resource and ownership usage.
use std::mem::MaybeUninit;
use unimemory::{Global, Heap, Memory, MemoryExt, Stack};

fn main() -> unimemory::Result<()> {
    let global = Global::new();
    let value = global.create(String::from("Hello, UniMemory!"))?;
    println!("{} ({})", *value, unimemory::BACKEND);

    let mut heap = Heap::new()?;
    {
        let values = heap.create_array(8, |index| index * index)?;
        assert_eq!(values[3], 9);
    }
    heap.reset()?;

    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    let mut stack = Stack::new(&mut buffer);
    {
        let mut bytes =
            stack.allocate_zeroed(std::alloc::Layout::from_size_align(128, 64).unwrap())?;
        bytes.fill(7);
        assert_eq!(bytes.bytes().unwrap()[0], 7);
    }
    stack.reset()?;
    assert_eq!(global.statistics().live_bytes, 0);
    Ok(())
}
