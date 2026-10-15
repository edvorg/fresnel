//! The bytes that the program holds on its heap: the memory of the soak. A count of the
//! allocator measures the data of the app alone, with no part of the allocator itself, so a
//! growth shows at once and the same in each run.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The allocator of the system with a count of the bytes in use.
pub(crate) struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The bytes in use now.
pub(crate) fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

// SAFETY: each call goes to the allocator of the system with the arguments of its caller,
// and the result goes back unchanged. The count only reads the sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller keeps the contract of `GlobalAlloc::alloc`.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller keeps the contract of `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller keeps the contract of `GlobalAlloc::realloc`.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Other tests of the program allocate at the same time, so the bounds have room.
    #[test]
    fn an_allocation_counts_until_its_drop() {
        const BLOCK: usize = 64 * 1024 * 1024;
        let before = live();
        let block = vec![1_u8; BLOCK];
        assert!(live() >= before + BLOCK / 2, "{} after {before}", live());
        drop(block);
        assert!(live() < before + BLOCK / 2, "{} after {before}", live());
    }
}
