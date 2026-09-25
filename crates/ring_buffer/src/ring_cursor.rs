use super::error::InitError;

/// The index arithmetic of a ring, without any storage.
///
/// Tracks the head, the length and the capacity, and hands out the physical
/// index to read or write for each operation. [`RingBuffer`](crate::RingBuffer)
/// is this cursor plus one `Box<[MaybeUninit<T>]>`; it is public so the same
/// arithmetic can drive a struct-of-arrays ring, one storage slice per field,
/// each of length [`Self::real_size`].
///
/// The capacity is rounded up to a power of two so an index wraps with a mask
/// instead of a division. The ring still reports full at the capacity that was
/// asked for.
#[derive(Debug, Clone, Copy)]
pub struct RingCursor {
    /// Storage length: the requested capacity rounded up to a power of two.
    pub(super) cap: usize,
    /// `cap - 1`: all low bits set, so `x & mask == x % cap`.
    pub(super) mask: usize,
    /// Capacity asked for, at which the ring reports full.
    pub(super) requested_cap: usize,

    pub(super) head: usize,
    pub(super) n_elem: usize,
}

impl RingCursor {
    /// Creates an empty cursor for `requested_capacity` elements.
    ///
    /// Fails on zero, or if rounding up to a power of two overflows.
    pub fn try_new(requested_capacity: usize) -> Result<Self, InitError> {
        if requested_capacity == 0 {
            return Err(InitError::ZeroCapacity);
        }

        let cap =
            requested_capacity
                .checked_next_power_of_two()
                .ok_or(InitError::CapacityOverflow {
                    asked_cap: requested_capacity,
                })?;

        Ok(Self {
            cap,
            mask: cap - 1,
            requested_cap: requested_capacity,

            head: 0,
            n_elem: 0,
        })
    }

    /// Number of live elements.
    pub fn len(&self) -> usize {
        self.n_elem
    }
    /// True when there is no live element.
    pub fn is_empty(&self) -> bool {
        self.n_elem == 0
    }
    /// True when the ring holds [`Self::asked_capacity`] elements.
    pub fn is_full(&self) -> bool {
        self.n_elem == self.requested_cap
    }
    /// The capacity asked for at construction.
    pub fn asked_capacity(&self) -> usize {
        self.requested_cap
    }
    /// The storage length: the asked capacity rounded up to a power of two.
    pub fn real_size(&self) -> usize {
        self.cap
    }
    /// Elements that can still be pushed before the ring is full.
    pub fn free_space(&self) -> usize {
        self.requested_cap - self.n_elem
    }

    pub(super) fn tail(&self) -> usize {
        (self.head + self.n_elem) & self.mask
    }

    /// Physical index of the `logical_index`th element from the head, or
    /// `None` past the last live element.
    pub fn checked_physical_index(&self, logical_index: usize) -> Option<usize> {
        // `then_some` rather than `then`: the arithmetic is cheap and has no
        // side effect, so computing it eagerly costs nothing.
        (logical_index < self.n_elem).then_some((self.head + logical_index) & self.mask)
    }

    // ------------------------------------------------------------------ push
    /// Reserves the slot before the head and returns its index, or `None` when
    /// the ring is full.
    pub fn try_push_head_index(&mut self) -> Option<usize> {
        (!self.is_full()).then(|| self.push_head_index())
    }
    /// Reserves the slot before the head and returns its index. When the ring
    /// is full the tail element falls out of the window: read it first if you
    /// need it.
    pub fn push_head_index(&mut self) -> usize {
        self.head = self.head.wrapping_sub(1) & self.mask;
        if !self.is_full() {
            self.n_elem += 1;
        }
        self.head
    }

    /// Reserves the slot after the tail and returns its index, or `None` when
    /// the ring is full.
    pub fn try_push_tail_index(&mut self) -> Option<usize> {
        (!self.is_full()).then(|| self.push_tail_index())
    }
    /// Reserves the slot after the tail and returns its index. When the ring is
    /// full the head element falls out of the window: read it first if you
    /// need it.
    pub fn push_tail_index(&mut self) -> usize {
        let idx_push = self.tail();
        if self.is_full() {
            self.head = self.head.wrapping_add(1) & self.mask;
        } else {
            self.n_elem += 1;
        }
        idx_push
    }

    // ------------------------------------------------------------------- pop
    /// Releases the head slot and returns its index, or `None` when empty.
    pub fn pop_head_index(&mut self) -> Option<usize> {
        self.n_elem = self.n_elem.checked_sub(1)?;
        let pop_idx = self.head;
        self.head = self.head.wrapping_add(1) & self.mask;
        Some(pop_idx)
    }

    /// Releases the tail slot and returns its index, or `None` when empty.
    pub fn pop_tail_index(&mut self) -> Option<usize> {
        self.n_elem = self.n_elem.checked_sub(1)?;
        Some(self.tail())
    }

    /// Splits `values` along the live window, oldest first: the second slice
    /// is non-empty only when the window wraps.
    ///
    /// Contract: `values.len() == self.real_size()`. The cursor drives an
    /// external storage (SoA) that must have exactly that length; it is
    /// checked in debug builds, and a shorter slice panics on slicing.
    pub fn as_slices<'a, T>(&self, values: &'a [T]) -> (&'a [T], &'a [T]) {
        debug_assert_eq!(
            values.len(),
            self.cap,
            "storage length must equal cursor capacity"
        );

        if self.is_empty() {
            return (&[], &[]);
        }

        let first_len = (self.cap - self.head).min(self.n_elem);
        (
            &values[self.head..self.head + first_len],
            &values[0..self.len() - first_len],
        )
    }

    /// Drops at most `by` elements from the head and returns how many.
    pub fn skip(&mut self, by: usize) -> usize {
        let max_skip = self.n_elem.min(by);
        self.head = self.head.wrapping_add(max_skip) & self.mask;
        self.n_elem -= max_skip;
        max_skip
    }
}

#[cfg(test)]
#[path = "tests/ring_cursor.rs"]
mod cursor_test;
