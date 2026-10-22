// A small size-class allocator. Interpreted programs allocate many short-lived
// small blocks (objects, strings, argument lists); recycling them through free
// lists is several times faster than the system heap, especially on Windows.
// Blocks above MAX_SMALL, or with unusual alignment, go to the system allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

const CLASSES: [usize; 12] = [16, 32, 48, 64, 80, 96, 128, 160, 192, 256, 384, 512];
const MAX_SMALL: usize = 512;
const CHUNK: usize = 256 * 1024;

struct Pools {
    free: [*mut u8; CLASSES.len()],
    /// Unused tail of the current chunk
    bump: *mut u8,
    bump_end: *mut u8,
}

pub struct Allocator {
    lock: AtomicBool,
    pools: UnsafeCell<Pools>,
}

// Access to `pools` is guarded by `lock`
unsafe impl Sync for Allocator {}

impl Allocator {
    pub const fn new() -> Allocator {
        Allocator {
            lock: AtomicBool::new(false),
            pools: UnsafeCell::new(Pools { free: [null_mut(); CLASSES.len()], bump: null_mut(), bump_end: null_mut() }),
        }
    }

    #[inline]
    fn class(size: usize) -> usize {
        // Classes are multiples of 16 up to 96, then coarser
        if size <= 96 {
            (size.max(1) + 15) / 16 - 1
        } else {
            CLASSES.iter().position(|&c| c >= size).unwrap()
        }
    }

    #[inline]
    fn with<T>(&self, f: impl FnOnce(&mut Pools) -> T) -> T {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            std::hint::spin_loop();
        }
        let r = f(unsafe { &mut *self.pools.get() });
        self.lock.store(false, Ordering::Release);
        r
    }
}

unsafe impl GlobalAlloc for Allocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > MAX_SMALL || layout.align() > 16 {
            return System.alloc(layout);
        }
        let c = Self::class(layout.size());
        let size = CLASSES[c];
        self.with(|p| {
            let head = p.free[c];
            if !head.is_null() {
                p.free[c] = *(head as *mut *mut u8);
                return head;
            }
            if (p.bump_end as usize) - (p.bump as usize) < size {
                let chunk = System.alloc(Layout::from_size_align_unchecked(CHUNK, 16));
                if chunk.is_null() {
                    return null_mut();
                }
                p.bump = chunk;
                p.bump_end = chunk.add(CHUNK);
            }
            let block = p.bump;
            p.bump = p.bump.add(size);
            block
        })
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() > MAX_SMALL || layout.align() > 16 {
            return System.dealloc(ptr, layout);
        }
        let c = Self::class(layout.size());
        self.with(|p| {
            *(ptr as *mut *mut u8) = p.free[c];
            p.free[c] = ptr;
        });
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let small = |s: usize| s <= MAX_SMALL && layout.align() <= 16;
        if !small(layout.size()) && !small(new_size) {
            return System.realloc(ptr, layout, new_size);
        }
        if small(layout.size()) && small(new_size) && Self::class(layout.size()) == Self::class(new_size) {
            return ptr; // still fits its block
        }
        let new = self.alloc(Layout::from_size_align_unchecked(new_size, layout.align()));
        if !new.is_null() {
            std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
            self.dealloc(ptr, layout);
        }
        new
    }
}
