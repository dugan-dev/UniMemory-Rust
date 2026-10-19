//! Same-binary native/API pairs share SDKs, flags and workloads. Results are advisory.
use allocator_api2::alloc::{AllocError, Allocator};
use std::mem::MaybeUninit;
use std::{alloc::Layout, hint::black_box, time::Instant};
use unimemory::benchmark::RawAllocator as NativeAllocator;
use unimemory::{Global, Heap, Memory, MemoryExt, Stack};

#[derive(Clone, Copy)]
struct Native;
// SAFETY: exactly requested sizes are reported; native allocation/deallocation pairs match.
unsafe impl Allocator for Native {
    #[inline]
    fn allocate(&self, layout: Layout) -> Result<std::ptr::NonNull<[u8]>, AllocError> {
        let pointer = if layout.size() == 0 {
            std::ptr::without_provenance_mut(layout.align())
        } else {
            // SAFETY: valid nonzero layout and matching deallocation below.
            unsafe { NativeAllocator.alloc(layout) }
        };
        std::ptr::NonNull::new(pointer)
            .map(|p| std::ptr::NonNull::slice_from_raw_parts(p, layout.size()))
            .ok_or(AllocError)
    }
    #[inline]
    unsafe fn deallocate(&self, pointer: std::ptr::NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            // SAFETY: caller supplies matching pointer/layout from this native allocator.
            unsafe {
                NativeAllocator.dealloc(pointer.as_ptr(), layout);
            }
        }
    }
}

fn measure(mut operation: impl FnMut(), count: usize) -> f64 {
    for _ in 0..10_000 {
        operation();
    }
    let start = Instant::now();
    for _ in 0..count {
        operation();
    }
    start.elapsed().as_secs_f64() * 1e9 / count as f64
}

fn paired(
    case: &str,
    bytes: usize,
    alignment: usize,
    count: usize,
    mut native: impl FnMut(),
    mut unified: impl FnMut(),
) {
    let mut native_trials = std::vec::Vec::new();
    let mut unified_trials = std::vec::Vec::new();
    for trial in 0..9 {
        if trial % 2 == 0 {
            native_trials.push(measure(&mut native, count));
            unified_trials.push(measure(&mut unified, count));
        } else {
            unified_trials.push(measure(&mut unified, count));
            native_trials.push(measure(&mut native, count));
        }
    }
    let mut n = native_trials.clone();
    let mut u = unified_trials.clone();
    n.sort_by(f64::total_cmp);
    u.sort_by(f64::total_cmp);
    for (trial, (native, unified)) in native_trials.iter().zip(&unified_trials).enumerate() {
        println!(
            "{},{case},{bytes},{alignment},{trial},{native:.4},{unified:.4},{:.2}",
            unimemory::BACKEND,
            native / unified * 100.0
        );
    }
    eprintln!(
        "{} {case} {bytes}B: native {:.2}ns / API {:.2}ns ({:.1}%)",
        unimemory::BACKEND,
        n[4],
        u[4],
        n[4] / u[4] * 100.0
    );
}

fn main() {
    let memory = Global::new();
    let count = std::env::var("UNIMEMORY_BENCH_ITERATIONS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(200_000);
    assert!(count > 0, "iteration count must be positive");
    println!("backend,case,bytes,alignment,trial,native_ns,unified_ns,native_over_unified_percent");
    for bytes in [16, 64, 256, 4096, 65536] {
        for alignment in [8, 64] {
            let layout = Layout::from_size_align(bytes, alignment).unwrap();
            paired(
                "global",
                bytes,
                alignment,
                count,
                || {
                    // SAFETY: valid native allocation pair; observe a written byte to retain the workload.
                    unsafe {
                        let pointer = NativeAllocator.alloc(black_box(layout));
                        assert!(!pointer.is_null());
                        black_box(pointer).write(17);
                        NativeAllocator.dealloc(black_box(pointer), layout);
                    }
                },
                || {
                    let pointer = memory.allocate_raw(black_box(layout), false).unwrap();
                    // SAFETY: exact live allocation, unique access and matching layout.
                    unsafe {
                        black_box(pointer.as_ptr()).write(17);
                        memory.deallocate_raw(black_box(pointer), layout);
                    }
                },
            );
        }
    }
    let layout = Layout::from_size_align(64, 16).unwrap();
    paired(
        "zeroed",
        64,
        16,
        count,
        || {
            // SAFETY: native zeroed allocation initializes the read byte; matching deallocation follows.
            unsafe {
                let p = NativeAllocator.alloc_zeroed(black_box(layout));
                assert!(!p.is_null());
                black_box(*black_box(p));
                NativeAllocator.dealloc(p, layout);
            }
        },
        || {
            let p = memory.allocate_raw(black_box(layout), true).unwrap();
            // SAFETY: initialized byte, live pointer and exact matching layout.
            unsafe {
                black_box(*black_box(p.as_ptr()));
                memory.deallocate_raw(p, layout);
            }
        },
    );
    let grown = Layout::from_size_align(512, 16).unwrap();
    paired(
        "resize",
        512,
        16,
        count,
        || {
            // SAFETY: valid native allocation, successful realloc replacement, and matching new layout.
            unsafe {
                let p = NativeAllocator.alloc(black_box(layout));
                assert!(!p.is_null());
                black_box(p).write(7);
                let p = NativeAllocator.realloc(black_box(p), layout, 512);
                assert!(!p.is_null());
                black_box(*black_box(p));
                NativeAllocator.dealloc(p, grown);
            }
        },
        || {
            let p = memory.allocate_raw(black_box(layout), false).unwrap();
            // SAFETY: exclusive ownership; old/new layouts match and the prefix byte is initialized.
            unsafe {
                black_box(p.as_ptr()).write(7);
                let p = memory
                    .reallocate_raw(black_box(p), layout, grown, false)
                    .unwrap();
                black_box(*black_box(p.as_ptr()));
                memory.deallocate_raw(p, grown);
            }
        },
    );
    let heap = Heap::new().unwrap();
    let native_heap = unimemory::benchmark::NativeHeap::new().unwrap();
    paired(
        "heap",
        64,
        16,
        count,
        || {
            let p = native_heap.allocate_raw(black_box(layout), false).unwrap();
            // SAFETY: matching native heap allocation, unique byte write and exact deallocation.
            unsafe {
                black_box(p.as_ptr()).write(17);
                native_heap.deallocate_raw(black_box(p), layout);
            }
        },
        || {
            let p = heap.allocate_raw(black_box(layout), false).unwrap();
            // SAFETY: matching resource, exact layout and unique live allocation.
            unsafe {
                black_box(p.as_ptr()).write(17);
                heap.deallocate_raw(black_box(p), layout);
            }
        },
    );
    paired(
        "object",
        64,
        8,
        count,
        || {
            let layout = Layout::new::<[u64; 8]>();
            // SAFETY: enough aligned storage for the value, initialized once, then freed natively.
            unsafe {
                let p = NativeAllocator.alloc(layout).cast::<[u64; 8]>();
                assert!(!p.is_null());
                p.write(black_box([17; 8]));
                black_box((*black_box(p))[0]);
                NativeAllocator.dealloc(p.cast(), layout);
            }
        },
        || {
            let value = memory.create(black_box([17u64; 8])).unwrap();
            black_box(std::ptr::from_ref(&*value));
            black_box(value[0]);
        },
    );
    paired(
        "vector",
        2048,
        8,
        count / 4 + 1,
        || {
            let mut values = unimemory::Vec::with_capacity_in(black_box(256), Native);
            values.extend(0u64..256);
            black_box(&values);
            black_box(values[255]);
        },
        || {
            let mut values = unimemory::Vec::with_capacity_in(black_box(256), memory.allocator());
            values.extend(0u64..256);
            black_box(&values);
            black_box(values[255]);
        },
    );
    let mut native_buffer = [MaybeUninit::<u8>::uninit(); 256];
    let native_start = black_box(native_buffer.as_mut_ptr().cast::<u8>());
    let native_capacity = black_box(native_buffer.len());
    let mut buffer = [MaybeUninit::<u8>::uninit(); 256];
    let mut stack = Stack::new(black_box(&mut buffer));
    paired(
        "stack",
        64,
        16,
        count,
        || {
            let request = black_box(layout);
            if request.size() == 0 {
                return;
            }
            let padding = native_start.addr().wrapping_neg() & (request.align() - 1);
            assert!(padding
                .checked_add(request.size())
                .is_some_and(|end| end <= native_capacity));
            // SAFETY: aligned in-bounds byte range in the native fixed-buffer control.
            unsafe {
                let p = native_start.add(padding);
                black_box(p).write(17);
                black_box(p);
            }
        },
        || {
            {
                let mut value = stack.allocate(black_box(layout)).unwrap();
                // SAFETY: this unique block owns the valid byte being written.
                unsafe {
                    black_box(value.as_mut_ptr()).write(17);
                }
                black_box(value.as_ptr());
            }
            stack.reset().unwrap();
        },
    );
}
