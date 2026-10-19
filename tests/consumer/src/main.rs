use std::mem::MaybeUninit;
use unimemory::{Global, Heap, Memory, MemoryExt, Stack, StatisticsMode, Vec};

fn main() -> unimemory::Result<()> {
    Global::config(100)?;
    let mut global = Global::with_statistics(StatisticsMode::Basic)?;
    {
        let value = global.create(String::from("public interface"))?;
        assert_eq!(&**value, "public interface");
        let mut values = Vec::new_in(global.allocator());
        values.extend(0u64..256);
        assert_eq!(values.iter().sum::<u64>(), 32640);
    }
    assert_eq!(global.statistics().live_bytes, 0);
    global.reset_statistics()?;
    let mut heap = Heap::new()?;
    {
        let memory: &dyn Memory = &heap;
        let values = memory.create_array(16, |i| i as u64)?;
        assert_eq!(values[15], 15);
    }
    heap.reset()?;
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    let mut stack = Stack::new(&mut buffer);
    let mark = stack.mark();
    {
        let value = stack.create(42)?;
        assert_eq!(*value, 42);
    }
    stack.rewind(mark)?;
    assert_eq!(stack.used(), 0);
    println!("public consumer passed: {}", unimemory::BACKEND);
    Ok(())
}
