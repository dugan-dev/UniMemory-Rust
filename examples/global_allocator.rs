//! Opt-in replacement for standard Rust containers' global allocation.
#[global_allocator]
static ALLOCATOR: unimemory::NativeAllocator = unimemory::NativeAllocator;

fn main() {
    let values: std::vec::Vec<_> = (0..1024).collect();
    let value = std::boxed::Box::new(String::from("native backend"));
    assert_eq!(values.len(), 1024);
    println!("{}: {}", unimemory::BACKEND, value);
}
