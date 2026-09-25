//! Classic Vyukov bounded queue, kept only as a benchmark baseline for
//! [`super`]: producers claim a slot with a CAS on `tail` and retry when they
//! lose, where the ticket ring takes a `fetch_add` that always succeeds.
//!
//! Deliberately minimal: no closure, no batching, no observers. It exists to be
//! measured against, not used.
//!
//! # Safety invariants
//!
//! 1. **Bounds.** Every buffer access goes through `idx & (N - 1)` with `N` a
//!    power of two, so the index is `< N == buff.len()`.
//! 2. **Slot ownership is carried by `seq`**, as in the ticket ring: for
//!    position `p`, `seq == p` means empty and owned by the producer whose CAS
//!    moved `tail` past `p`; `seq == p + 1` means filled and owned by the
//!    consumer; `seq == p + N` hands the slot to position `p + N`. Transitions
//!    are `Release` stores observed with `Acquire` loads.

use std::{mem::MaybeUninit, ops::Deref};

use crate::error::*;

use super::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    cell::UnsafeCell,
};

struct Cell<T> {
    data: UnsafeCell<MaybeUninit<T>>,
    seq: AtomicUsize,
}

impl<T> Cell<T> {
    fn new(seq: usize) -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
            seq: AtomicUsize::new(seq),
        }
    }
}

/// Shared state of the baseline queue.
pub struct Ring<T, const N: usize> {
    buff: Box<[Cell<T>; N]>,
    tail: AtomicUsize,
}

// SAFETY: values of `T` only move from producer threads to the consumer thread,
// and concurrent access to `data` is serialised by invariant 2.
unsafe impl<T: Send, const N: usize> Send for Ring<T, N> {}
// SAFETY: see the `Send` impl above.
unsafe impl<T: Send, const N: usize> Sync for Ring<T, N> {}

impl<T, const N: usize> Ring<T, N> {
    fn new() -> Self {
        const { assert!(N.is_power_of_two() && N >= 2, "N must be power-of-two >= 2") };

        Self {
            // Built on the stack then moved: fine at the capacities benchmarked.
            buff: Box::new(std::array::from_fn(Cell::new)),
            tail: AtomicUsize::new(0),
        }
    }
}

impl<T, const N: usize> Drop for Ring<T, N> {
    fn drop(&mut self) {
        let tail = self.tail.load(Ordering::Relaxed);
        for idx in (0..N).map(|i| tail.wrapping_sub(N).wrapping_add(i)) {
            let cell = &self.buff[idx & (N - 1)];
            if cell.seq.load(Ordering::Relaxed) == idx.wrapping_add(1) {
                // SAFETY: invariant 2: the slot holds an unread value, and
                // `&mut self` means no handle can reach it any more.
                unsafe { cell.data.with_mut(|ptr| (*ptr).assume_init_drop()) }
            }
        }
    }
}

/// Sending half of the baseline queue. Clone it to add producers.
pub struct Producer<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
}

impl<T, const N: usize> Deref for Producer<T, N> {
    type Target = Ring<T, N>;

    fn deref(&self) -> &Self::Target {
        &self.ring
    }
}

impl<T, const N: usize> Producer<T, N> {
    /// Pushes one value, retrying the CAS while other producers win it.
    /// Returns [`Error::ChannelFull`] with the value when no slot is free.
    pub fn push(&self, value: T) -> Result<(), Error<T>> {
        let mut pos = self.tail.load(Ordering::Relaxed);
        let cell = loop {
            let cell = &self.buff[pos & (N - 1)];
            let diff = cell.seq.load(Ordering::Acquire).wrapping_sub(pos) as isize;
            if diff == 0 {
                match self.tail.compare_exchange_weak(
                    pos,
                    pos.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break cell,
                    Err(actual) => pos = actual,
                }
            } else if diff < 0 {
                return Err(Error::ChannelFull(value));
            } else {
                pos = self.tail.load(Ordering::Relaxed);
            }
        };
        // SAFETY: invariant 2: `seq == pos` and our CAS claimed `pos`, so the
        // slot is empty and ours until the store below publishes it.
        unsafe {
            cell.data.with_mut(|ptr| (*ptr).write(value));
        }
        cell.seq.store(pos.wrapping_add(1), Ordering::Release);
        Ok(())
    }
}
impl<T, const N: usize> Clone for Producer<T, N> {
    fn clone(&self) -> Self {
        Producer {
            ring: Arc::clone(&self.ring),
        }
    }
}

/// Receiving half of the baseline queue.
pub struct Consumer<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
    head: usize,
}
impl<T, const N: usize> Consumer<T, N> {
    /// Takes the head item, or `None` if it is not published yet.
    pub fn pop(&mut self) -> Option<T> {
        let cell = &self.ring.buff[self.head & (N - 1)];
        if cell.seq.load(Ordering::Acquire) != self.head.wrapping_add(1) {
            return None;
        }
        // SAFETY: invariant 2: `seq == head + 1` was observed with `Acquire`,
        // so the slot holds a published value owned by the single consumer.
        let value = unsafe { cell.data.with(|c| (*c).assume_init_read()) };
        cell.seq.store(self.head.wrapping_add(N), Ordering::Release);
        self.head = self.head.wrapping_add(1);
        Some(value)
    }
}

/// Creates the baseline queue.
pub fn channel<T, const N: usize>() -> (Producer<T, N>, Consumer<T, N>) {
    let ring = Arc::new(Ring::new());
    (
        Producer {
            ring: Arc::clone(&ring),
        },
        Consumer { ring, head: 0 },
    )
}
