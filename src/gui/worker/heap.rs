//! The browser executable's counted heap.
//!
//! Every allocation is counted. A tab process sets a limit when it starts;
//! past it, allocation fails and Rust stops the process, so one page cannot
//! exhaust the computer's memory. The browser itself has no limit.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    ptr::null_mut,
    sync::atomic::{AtomicUsize, Ordering},
};

static IN_USE: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// The system allocator, counted. Install it as the executable's global
/// allocator so tab processes can limit their heap.
pub struct CountedHeap;

/// Limit the counted heap to `bytes`. Returns false when this executable does
/// not count its allocations, so no limit can apply.
pub(super) fn limit(bytes: usize) -> bool {
    LIMIT.store(bytes, Ordering::Relaxed);
    IN_USE.load(Ordering::Relaxed) > 0
}

/// Count `size` more bytes, unless that would pass the limit.
fn reserve(size: usize) -> bool {
    // A layout's size is at most isize::MAX, so the sum cannot overflow while
    // the bytes in use fit in memory.
    let used = IN_USE.fetch_add(size, Ordering::Relaxed) + size;
    if used > LIMIT.load(Ordering::Relaxed) {
        release(size);
        return false;
    }
    true
}

fn release(size: usize) {
    IN_USE.fetch_sub(size, Ordering::Relaxed);
}

// SAFETY: each method passes its caller's layout and pointer unchanged to the
// system allocator, which upholds the GlobalAlloc contract. The counters only
// decide whether to call it, and failing returns null as the contract allows.
unsafe impl GlobalAlloc for CountedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return null_mut();
        }
        // SAFETY: the caller upholds `alloc`'s contract for `layout`.
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            release(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return null_mut();
        }
        // SAFETY: the caller upholds `alloc_zeroed`'s contract for `layout`.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if pointer.is_null() {
            release(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller passes a block this allocator returned for `layout`.
        unsafe { System.dealloc(pointer, layout) };
        release(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let growth = new_size.saturating_sub(layout.size());
        if !reserve(growth) {
            return null_mut();
        }
        // SAFETY: the caller passes a block this allocator returned for
        // `layout` and a valid new size, as `realloc` requires.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if moved.is_null() {
            release(growth);
        } else {
            release(layout.size().saturating_sub(new_size));
        }
        moved
    }
}
