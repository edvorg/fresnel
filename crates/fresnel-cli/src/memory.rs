//! The bytes that the program holds on its heap: the memory of the soak. A count of the
//! allocator measures the data of the app alone, with no part of the allocator itself, so a
//! growth shows at once and the same in each run.
//!
//! Each thread also counts the bytes that it allocates. The memory account of the core
//! measures a part whose size it does not know as the bytes of a copy that this thread
//! makes (`Copies`): no other thread adds to that count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use fresnel_core::memory::Scale;

/// The allocator of the system with a count of the bytes in use.
pub(crate) struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The bytes that this thread allocated since its start. A free does not count: a copy
    /// allocates the bytes of its value, and its drop comes after the measure.
    static OWN: Cell<usize> = const { Cell::new(0) };
}

/// The bytes in use now.
pub(crate) fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// Adds to the count of this thread. A thread local with a constant value and no drop has
/// no state of its own to set up, so the allocator can read it.
fn own(bytes: usize) {
    OWN.with(|own| own.set(own.get().wrapping_add(bytes)));
}

/// The scale of the memory account: the bytes that a copy allocates on this thread.
pub(crate) struct Copies;

impl Scale for Copies {
    fn bytes(&self, copy: &mut dyn FnMut()) -> usize {
        let before = OWN.with(Cell::get);
        copy();
        OWN.with(Cell::get).wrapping_sub(before)
    }
}

// SAFETY: each call goes to the allocator of the system with the arguments of its caller,
// and the result goes back unchanged. The count only reads the sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller keeps the contract of `GlobalAlloc::alloc`.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            own(layout.size());
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
            own(new_size.saturating_sub(layout.size()));
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

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

    /// The other threads of the test program allocate at the same time and add nothing.
    #[test]
    fn a_copy_measures_the_bytes_of_its_value_on_its_thread() {
        let words: Vec<String> = (0..100).map(|number| format!("{number:0>40}")).collect();
        let bytes = Copies.bytes(&mut || drop(black_box(words.clone())));
        assert_eq!(bytes, 100 * size_of::<String>() + 100 * 40);
        assert_eq!(Copies.bytes(&mut || {}), 0);
    }
}
