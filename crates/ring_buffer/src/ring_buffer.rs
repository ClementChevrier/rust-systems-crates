//! # Safety invariants
//!
//! Every `unsafe` block in this file relies on one or more of these.
//!
//! 1. **Bounds.** Every index the cursor hands out is masked with
//!    `cap - 1`, where `cap == inner.len()` is a power of two, so it is
//!    `< inner.len()`.
//! 2. **Initialisation.** Exactly the slots in the live window, the `len`
//!    slots from `head` (wrapping), are initialised. A push initialises the
//!    slot the cursor just added to the window; a pop or an eviction moves the
//!    value out of the slot the cursor just removed. The cursor's
//!    `checked_physical_index` only returns indices inside the window.
//! 3. **Layout.** `MaybeUninit<T>` is `repr(transparent)` over `T`, so a
//!    slice of initialised `MaybeUninit<T>` can be viewed as a `[T]`.

use std::{
    fmt::{Debug, Display},
    mem::MaybeUninit,
};

use super::{error::InitError, ring_cursor::RingCursor};

/// Fixed-capacity double-ended ring buffer.
///
/// Push and pop at both ends, with three flavours of push:
/// `try_push_*` refuses when full, `push_*` overwrites the opposite end, and
/// `push_*_evicting` returns what it overwrote.
pub struct RingBuffer<T> {
    inner: Box<[MaybeUninit<T>]>,
    cursor: RingCursor,
}

impl<T> RingBuffer<T> {
    /// Creates an empty ring that reports full at `requested_capacity`.
    ///
    /// Unlike `VecDeque`, the capacity is fixed at construction. The storage is
    /// rounded up to the next power of two. Fails on zero or if that rounding
    /// overflows.
    pub fn try_new(requested_capacity: usize) -> Result<Self, InitError> {
        let cursor = RingCursor::try_new(requested_capacity)?;
        let storage_capacity = cursor.real_size();

        Ok(Self {
            cursor,
            inner: Box::new_uninit_slice(storage_capacity),
        })
    }

    /// Number of elements in the ring.
    pub fn len(&self) -> usize {
        self.cursor.len()
    }
    /// True when the ring holds no element.
    pub fn is_empty(&self) -> bool {
        self.cursor.is_empty()
    }
    /// True when the ring holds [`Self::asked_capacity`] elements.
    pub fn is_full(&self) -> bool {
        self.cursor.is_full()
    }
    /// The capacity asked for at construction.
    pub fn asked_capacity(&self) -> usize {
        self.cursor.asked_capacity()
    }
    /// The allocated storage: the asked capacity rounded up to a power of two.
    pub fn real_size(&self) -> usize {
        self.cursor.real_size()
    }
    /// Elements that can still be pushed before the ring is full.
    pub fn free_space(&self) -> usize {
        self.cursor.free_space()
    }

    /// Borrows the `i`th element from the head without moving the ring.
    pub fn peek(&self, i: usize) -> Option<&T> {
        let idx = self.cursor.checked_physical_index(i)?;
        // SAFETY: invariants 1 and 2: the index is in bounds and inside the
        // live window.
        Some(unsafe { self.inner.get_unchecked(idx).assume_init_ref() })
    }
    /// Mutably borrows the `i`th element from the head without moving the ring.
    pub fn peek_mut(&mut self, i: usize) -> Option<&mut T> {
        let idx = self.cursor.checked_physical_index(i)?;
        // SAFETY: invariants 1 and 2, as in `peek`.
        Some(unsafe { self.inner.get_unchecked_mut(idx).assume_init_mut() })
    }
    /// Removes and returns the element at the head.
    pub fn pop_head(&mut self) -> Option<T> {
        let idx_pop = self.cursor.pop_head_index()?;
        // SAFETY: invariant 1; invariant 2: the slot was the head of the window
        // and the cursor has just released it, so it is read exactly once.
        Some(unsafe { self.inner.get_unchecked(idx_pop).assume_init_read() })
    }
    /// Removes and returns the element at the tail.
    pub fn pop_tail(&mut self) -> Option<T> {
        let idx_pop = self.cursor.pop_tail_index()?;
        // SAFETY: invariant 1; invariant 2: the slot was the tail of the window
        // and the cursor has just released it, so it is read exactly once.
        Some(unsafe { self.inner.get_unchecked(idx_pop).assume_init_read() })
    }

    // ------------------------------------------------------------- push head
    fn push_head_inner(&mut self, value: T) -> Option<T> {
        let evicted = self.cursor.is_full().then(|| {
            let tail = (self.cursor.head + self.cursor.n_elem - 1) & self.cursor.mask;
            // SAFETY: invariant 1; invariant 2: the ring is full, so the tail
            // slot is initialised. The push below takes it out of the window,
            // so the value is moved out exactly once.
            unsafe { self.inner.get_unchecked(tail).assume_init_read() }
        });
        let idx = self.cursor.push_head_index();
        // SAFETY: invariant 1. Invariant 2: the cursor has just added this slot
        // to the window, and it is either free or the evicted slot read above.
        unsafe {
            self.inner.get_unchecked_mut(idx).write(value);
        }
        evicted
    }

    /// Pushes at the head, or hands the value back in `Err` when the ring is
    /// full.
    ///
    /// See [`RingBuffer::push_head`] to overwrite instead.
    pub fn try_push_head(&mut self, value: T) -> Result<(), T> {
        match self.cursor.try_push_head_index() {
            None => Err(value),
            Some(idx) => {
                // SAFETY: invariant 1; invariant 2: the slot was just added to
                // the window and is free.
                unsafe {
                    self.inner.get_unchecked_mut(idx).write(value);
                }
                Ok(())
            }
        }
    }

    /// Pushes at the head, dropping the tail element when the ring is full.
    ///
    /// See [`RingBuffer::push_head_evicting`] to get the dropped element back,
    /// and [`RingBuffer::try_push_head`] to refuse instead.
    pub fn push_head(&mut self, value: T) {
        drop(self.push_head_inner(value));
    }
    /// Pushes at the head and returns the tail element it evicted, if any.
    ///
    /// See [`RingBuffer::push_head`] when the evicted element is not needed,
    /// and [`RingBuffer::try_push_head`] to refuse instead.
    pub fn push_head_evicting(&mut self, value: T) -> Option<T> {
        self.push_head_inner(value)
    }

    // ------------------------------------------------------------- push tail
    fn push_tail_inner(&mut self, value: T) -> Option<T> {
        let evicted = self.cursor.is_full().then(|| {
            // SAFETY: invariant 1; invariant 2: the ring is full, so the head
            // slot is initialised. The push below takes it out of the window,
            // so the value is moved out exactly once.
            unsafe {
                self.inner
                    .get_unchecked(self.cursor.head)
                    .assume_init_read()
            }
        });
        let idx = self.cursor.push_tail_index();
        // SAFETY: invariant 1. Invariant 2: the cursor has just added this slot
        // to the window, and it is either free or the evicted slot read above.
        unsafe {
            self.inner.get_unchecked_mut(idx).write(value);
        }
        evicted
    }

    /// Pushes at the tail, or hands the value back in `Err` when the ring is
    /// full.
    ///
    /// See [`RingBuffer::push_tail`] to overwrite instead.
    pub fn try_push_tail(&mut self, value: T) -> Result<(), T> {
        match self.cursor.try_push_tail_index() {
            None => Err(value),
            Some(idx) => {
                // SAFETY: invariant 1; invariant 2: the slot was just added to
                // the window and is free.
                unsafe {
                    self.inner.get_unchecked_mut(idx).write(value);
                }
                Ok(())
            }
        }
    }

    /// Pushes at the tail, dropping the head element when the ring is full.
    ///
    /// See [`RingBuffer::push_tail_evicting`] to get the dropped element back,
    /// and [`RingBuffer::try_push_tail`] to refuse instead.
    pub fn push_tail(&mut self, value: T) {
        drop(self.push_tail_inner(value));
    }

    /// Pushes at the tail and returns the head element it evicted, if any.
    ///
    /// See [`RingBuffer::push_tail`] when the evicted element is not needed,
    /// and [`RingBuffer::try_push_tail`] to refuse instead.
    pub fn push_tail_evicting(&mut self, value: T) -> Option<T> {
        self.push_tail_inner(value)
    }

    /// The live window as one or two slices, head first, with no copy.
    ///
    /// When the window does not wrap, the first slice runs from `head` to
    /// `tail` and the second is empty:
    ///
    /// ```text
    ///     head           tail
    ///      |               |
    ///   |----------------------|
    /// ```
    ///
    /// When it wraps, the first slice runs from `head` to the end of the
    /// storage and the second from the start of the storage to `tail`:
    ///
    /// ```text
    ///              tail   head
    ///               |       |
    ///   |----------------------|
    /// ```
    pub fn as_slices(&self) -> (&[T], &[T]) {
        let (first, second) = self.cursor.as_slices(self.inner.as_ref());

        // SAFETY: invariant 2: the cursor only slices the live window, whose
        // slots are all initialised. Invariant 3 makes the pointer cast valid,
        // and the returned slices borrow `&self`.
        unsafe {
            (
                std::slice::from_raw_parts(first.as_ptr() as *const T, first.len()),
                std::slice::from_raw_parts(second.as_ptr() as *const T, second.len()),
            )
        }
    }

    /// Iterates from the head to the tail. Reversible.
    pub fn iter_head_to_tail(&self) -> impl DoubleEndedIterator<Item = &T> {
        let (first, second) = self.as_slices();
        first.iter().chain(second.iter())
    }

    /// Lazily removes elements from the head, front to back.
    ///
    /// Elements not drained stay in the ring. The iterator borrows the ring
    /// mutably, so it cannot be used while draining:
    ///
    /// ```compile_fail,E0499
    /// use ring_buffer::RingBuffer;
    /// let mut buffer = RingBuffer::try_new(6).expect("requested capacity must be valid");
    /// let drain = buffer.drain_from_head();
    /// buffer.push_head(3);
    /// drain.next();
    /// ```
    #[must_use = "the iterator is lazy: dropping it without iterating consumes nothing"]
    pub fn drain_from_head(&mut self) -> DrainRingFront<'_, T> {
        DrainRingFront::new(self)
    }

    /// Drops at most `by` elements from the head and returns how many.
    pub fn skip(&mut self, by: usize) -> usize {
        if std::mem::needs_drop::<T>() {
            let max_skip = self.len().min(by);
            for _ in 0..max_skip {
                self.pop_head();
            }
            max_skip
        } else {
            self.cursor.skip(by)
        }
    }

    /// Drops every element.
    pub fn clear(&mut self) {
        while self.pop_head().is_some() {}
    }
}
impl<T: Copy> RingBuffer<T> {
    /// Copies out the `i`th element from the head.
    pub fn get(&self, i: usize) -> Option<T> {
        let idx = self.cursor.checked_physical_index(i)?;
        // SAFETY: invariants 1 and 2: the index is in bounds and inside the
        // live window. `T: Copy`, so reading does not move out of the slot.
        Some(unsafe { self.inner.get_unchecked(idx).assume_init() })
    }
}

impl<T: Debug> std::fmt::Debug for RingBuffer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RingBuffer {{ len: {}, asked_capacity: {}, real_capacity: {}, data: [",
            self.len(),
            self.asked_capacity(),
            self.cursor.real_size()
        )?;
        for elem in self.iter_head_to_tail() {
            write!(f, "{elem:?}, ")?;
        }
        write!(f, "] }}")
    }
}

/// Past this many elements, `Display` shows the first and last three only.
const DISPLAY_FULL_LEN: usize = 6;
const DISPLAY_EDGE_LEN: usize = 3;

impl<T: Display> std::fmt::Display for RingBuffer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.len();
        write!(
            f,
            "RingBuffer {{ len: {len}, capacity: {}, data: [",
            self.asked_capacity()
        )?;

        if len > DISPLAY_FULL_LEN {
            for value in self.iter_head_to_tail().take(DISPLAY_EDGE_LEN) {
                write!(f, "{value}, ")?;
            }
            write!(f, "...")?;
            for value in self.iter_head_to_tail().skip(len - DISPLAY_EDGE_LEN) {
                write!(f, ", {value}")?;
            }
        } else {
            for (i, value) in self.iter_head_to_tail().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{value}")?;
            }
        }
        write!(f, "] }}")
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        if std::mem::needs_drop::<T>() {
            while self.pop_head().is_some() {}
        }
    }
}

impl<T: Clone> Clone for RingBuffer<T> {
    fn clone(&self) -> Self {
        let mut cursor = self.cursor;
        cursor.head = 0;
        cursor.n_elem = 0;
        let mut ring = RingBuffer {
            inner: Box::new_uninit_slice(cursor.cap),
            cursor,
        };
        for v in self.iter_head_to_tail() {
            ring.push_tail(v.clone());
        }
        ring
    }
}

/// Draining iterator returned by [`RingBuffer::drain_from_head`].
pub struct DrainRingFront<'a, T> {
    // Needed to release each slot from the window as its value is moved out.
    ring: &'a mut RingBuffer<T>,

    // The live window when the drain started, as raw slices: `as_slices`
    // cannot be kept as references while `ring` is borrowed mutably.
    first: *const T,
    first_len: usize,

    second: *const T,
    second_len: usize,
}
impl<'a, T> DrainRingFront<'a, T> {
    fn new(ring: &'a mut RingBuffer<T>) -> Self {
        let (first, second) = ring.as_slices();
        let (first, first_len) = (first.as_ptr(), first.len());
        let (second, second_len) = (second.as_ptr(), second.len());

        Self {
            ring,

            first,
            first_len,

            second,
            second_len,
        }
    }
}
impl<T> Iterator for DrainRingFront<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.first_len != 0 {
            // SAFETY: `first` points at the head of the live window (invariant
            // 2); the element is read once and immediately released by
            // `skip(1)`, which moves the head past it. The pointer then moves to
            // the next element of the same slice.
            let value = unsafe { self.first.read() };
            // SAFETY: at most one past the end of the `first` slice.
            self.first = unsafe { self.first.add(1) };
            self.first_len -= 1;
            self.ring.cursor.skip(1);
            Some(value)
        } else if self.second_len != 0 {
            // SAFETY: same reasoning as above, for the wrapped part of the
            // window.
            let value = unsafe { self.second.read() };
            // SAFETY: at most one past the end of the `second` slice.
            self.second = unsafe { self.second.add(1) };
            self.second_len -= 1;
            self.ring.cursor.skip(1);
            Some(value)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let total_len = self.first_len + self.second_len;
        (total_len, Some(total_len))
    }
}
impl<T> ExactSizeIterator for DrainRingFront<'_, T> {}

#[cfg(test)]
#[path = "tests/ring_buffer.rs"]
mod buffer_test;
