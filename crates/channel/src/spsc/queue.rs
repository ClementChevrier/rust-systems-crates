//! # Safety invariants
//!
//! Every `unsafe` block in this file relies on one or more of these.
//!
//! 1. **Bounds.** Every buffer access goes through `idx & (N - 1)`. `N` is a
//!    power of two (checked at compile time in `Ring::new`), so this equals
//!    `idx % N`, which is `< N == buf.len()`.
//! 2. **Ownership.** Slots in `[head, tail)` are initialised and belong to the
//!    consumer; slots in `[tail, head + N)` are uninitialised and belong to the
//!    producer. Only the producer advances `tail`, with a `Release` store made
//!    after writing the slots; only the consumer advances `head`, with a
//!    `Release` store made after reading them. Each side loads the other's index
//!    with `Acquire` before touching a slot, so every write to a slot
//!    happens-before the read that consumes it, and every read happens-before
//!    the write that reuses it.
//! 3. **Layout.** `Cell<T>` is `repr(transparent)` over
//!    `UnsafeCell<MaybeUninit<T>>`, itself layout-identical to `T`, so a
//!    `*const Cell<T>` cast to `*const T` addresses slot `i` at `.add(i)`. This
//!    only holds with the std `UnsafeCell`: loom's carries tracking state, which
//!    is why the `memcpy` paths are compiled out under `cfg(loom)`. Writing
//!    through a pointer derived from a shared reference is allowed because the
//!    bytes live inside an `UnsafeCell`.

use std::{mem::MaybeUninit, ops::Deref};

use crate::{error::*, util::CachePadded};

use super::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    cell::UnsafeCell,
};

#[repr(transparent)]
struct Cell<T> {
    data: UnsafeCell<MaybeUninit<T>>,
}
impl<T> Cell<T> {
    fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}
impl<T> Deref for Cell<T> {
    type Target = UnsafeCell<MaybeUninit<T>>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

struct Ring<T, const N: usize> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    closed: AtomicBool,
    buf: Box<[Cell<T>; N]>,
}

// SAFETY: the ring only moves `T` values from the producer thread to the
// consumer thread, so `T: Send` is the only requirement. Shared access to the
// cells is serialised by invariant 2: `Producer` and `Consumer` are the only
// handles, neither is `Clone`, and each touches only the slots it owns.
unsafe impl<T: Send, const N: usize> Send for Ring<T, N> {}
// SAFETY: see the `Send` impl above; no `&T` is ever shared between threads.
unsafe impl<T: Send, const N: usize> Sync for Ring<T, N> {}

impl<T, const N: usize> Ring<T, N> {
    fn new() -> Self {
        const { assert!(N.is_power_of_two() && N >= 2, "N must be power-of-two >= 2") };
        const {
            assert!(
                N <= (usize::MAX / 2 + 1),
                "N must not exceed half the usize range"
            )
        };

        // `Box::new(std::array::from_fn(..))` would build the array on the
        // stack first and then move it, which overflows the stack for a large
        // `N`. Building it in place on the heap avoids the copy.
        let mut heap_box: Box<[MaybeUninit<Cell<T>>]> = Box::new_uninit_slice(N);
        heap_box.iter_mut().for_each(|slot| {
            slot.write(Cell::<T>::new());
        });
        // SAFETY: the loop above initialised all `N` elements.
        let buf: Box<[Cell<T>; N]> = unsafe { heap_box.assume_init() }
            .try_into()
            .unwrap_or_else(|_| unreachable!("boxed slice always has length N"));

        Self {
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            closed: AtomicBool::new(false),
            buf,
        }
    }
}

impl<T, const N: usize> Drop for Ring<T, N> {
    fn drop(&mut self) {
        let tail = self.tail.load(Ordering::Acquire);
        let mut head = self.head.load(Ordering::Acquire);
        while head != tail {
            // SAFETY: invariant 1 for the index; invariant 2: `[head, tail)`
            // holds initialised values nobody has read, and `&mut self` means
            // both handles are gone, so each is dropped exactly once.
            unsafe {
                self.buf
                    .get_unchecked_mut(head & (N - 1))
                    .with_mut(|c| (*c).assume_init_drop());
            }
            head = head.wrapping_add(1);
        }
    }
}

/// Sending half of an SPSC channel.
///
/// Producer uniqueness is structural: it is not `Clone`.
///
/// ```compile_fail,E0599
/// let (tx, _rx) = channel::spsc::channel::<u64, 4>();
/// let _tx2 = tx.clone();
/// ```
pub struct Producer<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
    // Private copy of `tail`: only this handle writes it, so it never needs to
    // be read back from the atomic.
    tail: usize,
    // Last `head` observed. Reloaded only when this copy says the ring is full.
    cached_head: usize,
}
impl<T, const N: usize> std::fmt::Debug for Producer<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("spsc::Producer")
            .field("capacity", &N)
            .field("len", &self.len())
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl<T, const N: usize> Producer<T, N> {
    // ------------------------------------------------------ protocol internals
    fn new(ring: Arc<Ring<T, N>>) -> Self {
        Self {
            ring,
            tail: 0,
            cached_head: 0,
        }
    }

    fn write_cell(&mut self, idx: usize, value: T) {
        // SAFETY: invariant 1 for the index. Invariant 2: callers only pass
        // indices in `[tail, head + N)`, so the slot is free and owned by the
        // producer until the `Release` store of `tail` publishes it.
        unsafe {
            self.ring
                .buf
                .get_unchecked(idx & (N - 1))
                .with_mut(|c| (*c).write(value));
        }
    }

    fn increment_tail(&mut self, by: usize) {
        self.tail = self.tail.wrapping_add(by);
        self.ring.tail.store(self.tail, Ordering::Release);
    }

    /// True once either end has been dropped or closed.
    pub fn is_closed(&self) -> bool {
        self.ring.closed.load(Ordering::Acquire)
    }

    // ----------------------------------------------------------------- pushing
    /// Pushes one value.
    ///
    /// Fails fast: [`Error::ChannelFull`] when no slot is free,
    /// [`Error::ReceiverDisconnected`] once the channel is closed. The value
    /// comes back in both cases.
    pub fn push(&mut self, value: T) -> Result<(), Error<T>> {
        if self.is_closed() {
            return Err(Error::disconnected(value));
        }
        if self.tail.wrapping_sub(self.cached_head) == N {
            // Full according to the cached head: reload it and check again.
            self.cached_head = self.ring.head.load(Ordering::Acquire);
            if self.tail.wrapping_sub(self.cached_head) == N {
                return Err(Error::ChannelFull(value));
            }
        }

        self.write_cell(self.tail, value);
        self.increment_tail(1);
        Ok(())
    }

    /// Copies as many leading elements of `batch` as fit and returns how many
    /// were pushed. The whole batch is published with a single release store.
    ///
    /// Returns `Ok(0)` when the ring is full **or** the batch is empty: in both
    /// cases nothing was pushed. The waiting policy belongs to the caller.
    ///
    /// `T: Copy` because the elements are duplicated bit for bit: for a type
    /// that owns heap memory that would create two owners of one allocation.
    pub fn push_batch(&mut self, batch: &[T]) -> Result<usize, ReceiverDisconnected<()>>
    where
        T: Copy,
    {
        if self.is_closed() {
            return Err(ReceiverDisconnected::default());
        }

        // One extra load per batch is cheap next to the copy.
        self.cached_head = self.ring.head.load(Ordering::Acquire);
        let n_to_push = (N - self.tail.wrapping_sub(self.cached_head)).min(batch.len());

        if n_to_push == 0 {
            return Ok(0);
        }

        // The wrap is split into two contiguous runs so the compiler emits two
        // `memcpy`; with the mask inside a loop it cannot prove contiguity.
        #[cfg(not(loom))]
        {
            let start = self.tail & (N - 1);
            let first = n_to_push.min(N - start);
            let second = n_to_push - first;
            // SAFETY: invariant 3 makes `destination` a valid `*mut T` over the
            // `N` slots. Invariant 1: `start + first <= N` and `second <= start`
            // (because `n_to_push <= N`). Invariant 2: the `n_to_push` slots
            // from `tail` are free and owned by the producer. `batch` cannot
            // overlap the ring, which it does not own.
            unsafe {
                let destination = self.ring.buf.as_ptr() as *mut T;
                std::ptr::copy_nonoverlapping(batch.as_ptr(), destination.add(start), first);
                std::ptr::copy_nonoverlapping(batch.as_ptr().add(first), destination, second);
            }
        }
        // loom's `UnsafeCell` is not layout-transparent (invariant 3), so the
        // model goes through the cells one by one.
        #[cfg(loom)]
        {
            for (idx, elem) in batch[..n_to_push].iter().enumerate() {
                self.write_cell(self.tail.wrapping_add(idx), *elem);
            }
        }
        self.increment_tail(n_to_push);
        Ok(n_to_push)
    }

    /// Moves items out of `iter` until the ring is full or the iterator ends,
    /// and returns how many were pushed.
    ///
    /// Everything written is published in one release store. If the iterator
    /// panics mid-run, the items yielded before the panic are still published
    /// and the panic propagates: nothing leaks, nothing is lost.
    pub fn push_iter(
        &mut self,
        mut iter: impl Iterator<Item = T>,
    ) -> Result<usize, ReceiverDisconnected<()>> {
        if self.is_closed() {
            return Err(ReceiverDisconnected::default());
        }
        self.cached_head = self.ring.head.load(Ordering::Acquire);
        let free = N - self.tail.wrapping_sub(self.cached_head);

        let mut run = PublishRun {
            producer: self,
            pushed: 0,
        };
        while run.pushed < free {
            match iter.next() {
                Some(value) => {
                    run.producer
                        .write_cell(run.producer.tail.wrapping_add(run.pushed), value);
                    run.pushed += 1;
                }
                None => break,
            }
        }
        let pushed = run.pushed;
        drop(run);
        Ok(pushed)
    }

    // --------------------------------------------------------------- observers
    /// True when every slot is taken, as of the last `head` the consumer published.
    pub fn is_full(&self) -> bool {
        N == self.len()
    }

    /// Items currently in the ring. May lag behind a consumer that is popping.
    pub fn len(&self) -> usize {
        self.tail
            .wrapping_sub(self.ring.head.load(Ordering::Relaxed))
    }

    /// True when the consumer has read everything pushed so far.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Free slots, as of the last `head` the consumer published.
    pub fn free_space(&self) -> usize {
        N - self.len()
    }

    /// The compile-time capacity `N`.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Closes the channel. Items already pushed stay readable by the consumer.
    pub fn close(&mut self) {
        self.ring.closed.store(true, Ordering::Release);
    }
}
impl<T, const N: usize> Drop for Producer<T, N> {
    fn drop(&mut self) {
        self.ring.closed.store(true, Ordering::Release);
    }
}

/// Publishes on drop everything written so far, including during an unwind
/// out of `iter.next()`: no written item can leak.
struct PublishRun<'a, T, const N: usize> {
    producer: &'a mut Producer<T, N>,
    pushed: usize,
}
impl<T, const N: usize> Drop for PublishRun<'_, T, N> {
    fn drop(&mut self) {
        if self.pushed > 0 {
            self.producer.increment_tail(self.pushed);
        }
    }
}

/// Receiving half of an SPSC channel. Not `Clone` either.
pub struct Consumer<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
    // Private copy of `head`: only this handle writes it.
    head: usize,
    // Last `tail` observed. Reloaded only when this copy says the ring is empty.
    cached_tail: usize,
}
impl<T, const N: usize> std::fmt::Debug for Consumer<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("spsc::Consumer")
            .field("capacity", &N)
            .field("len", &self.len())
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl<T, const N: usize> Consumer<T, N> {
    // ------------------------------------------------------ protocol internals
    fn new(ring: Arc<Ring<T, N>>) -> Self {
        Self {
            ring,
            head: 0,
            cached_tail: 0,
        }
    }

    fn pop_cell(&mut self, idx: usize) -> T {
        // SAFETY: invariant 1 for the index. Invariant 2: callers only pass
        // indices in `[head, cached_tail)`, and `cached_tail` was loaded with
        // `Acquire`, so the slot is initialised. It is read once, then `head`
        // moves past it.
        unsafe {
            self.ring
                .buf
                .get_unchecked(idx & (N - 1))
                .with(|c| (*c).assume_init_read())
        }
    }

    fn update_state_after_pop(&mut self, by: usize) {
        self.head = self.head.wrapping_add(by);
        self.ring.head.store(self.head, Ordering::Release);
    }

    fn check_if_empty(&mut self) -> bool {
        if self.head == self.cached_tail {
            // Empty according to the cached tail: reload it and check again.
            self.cached_tail = self.ring.tail.load(Ordering::Acquire);
            self.head == self.cached_tail
        } else {
            false
        }
    }

    // ----------------------------------------------------- non-consuming reads
    /// Borrows the oldest item without consuming it.
    ///
    /// The returned reference borrows the consumer: `pop` and `drain` are
    /// rejected at compile time for as long as it lives.
    ///
    /// ```compile_fail,E0499
    /// let (mut tx, mut rx) = channel::spsc::channel::<u64, 4>();
    /// tx.push(1).unwrap();
    /// let peeked = rx.peek();
    /// let popped = rx.pop();          // E0499: `rx` is still borrowed by `peeked`
    /// assert_eq!(peeked, Some(&1));   // the borrow is still alive here
    /// ```
    pub fn peek(&mut self) -> Option<&T> {
        if self.check_if_empty() {
            return None;
        }

        Some(
            // SAFETY: invariant 1 for the index; invariant 2: `head <
            // cached_tail`, so the slot is initialised. The reference borrows
            // `&mut self`, so `head` cannot move and the slot cannot be read out
            // or reused while it lives.
            unsafe {
                self.ring
                    .buf
                    .get_unchecked(self.head & (N - 1))
                    .with(|cell| (*cell).assume_init_ref())
            },
        )
    }

    /// Opens a zero-copy view of everything published right now.
    ///
    /// See [`ReadChunk`]. Not available under loom (invariant 3).
    #[cfg(not(loom))]
    pub fn read_chunk(&mut self) -> ReadChunk<'_, T, N> {
        self.cached_tail = self.ring.tail.load(Ordering::Acquire);
        let len = self.cached_tail.wrapping_sub(self.head);
        ReadChunk {
            consumer: self,
            len,
        }
    }

    // --------------------------------------------------------- consuming reads
    /// Takes the oldest item, or `None` if nothing is published.
    pub fn pop(&mut self) -> Option<T> {
        if self.check_if_empty() {
            return None;
        }

        let value = self.pop_cell(self.head);
        self.update_state_after_pop(1);
        Some(value)
    }

    /// Iterator over the items published when `drain` is called.
    ///
    /// Items pushed while draining are left for the next call, so a fast
    /// producer cannot keep the loop running forever. Each item's slot is
    /// released as soon as it is yielded.
    #[must_use = "the iterator is lazy: dropping it without iterating consumes nothing"]
    pub fn drain(&mut self) -> Drain<'_, T, N> {
        self.cached_tail = self.ring.tail.load(Ordering::Acquire);
        let remaining = self.cached_tail.wrapping_sub(self.head);
        Drain {
            consumer: self,
            remaining,
        }
    }

    /// Copies up to `out.len()` items into `out` and returns how many.
    ///
    /// All the slots are released with one release store. `T: Copy` for the
    /// same reason as [`Producer::push_batch`].
    pub fn pop_batch(&mut self, out: &mut [T]) -> usize
    where
        T: Copy,
    {
        self.cached_tail = self.ring.tail.load(Ordering::Acquire);
        let n_to_pop = self.cached_tail.wrapping_sub(self.head).min(out.len());
        if n_to_pop == 0 {
            return 0;
        }

        // Two contiguous runs, as in `push_batch`.
        #[cfg(not(loom))]
        {
            let start = self.head & (N - 1);
            let first = n_to_pop.min(N - start);
            let second = n_to_pop - first;
            // SAFETY: invariant 3 for the cast. Invariant 1: `start + first <=
            // N`, `second <= start`. Invariant 2: the `n_to_pop` slots from
            // `head` are published and owned by the consumer. `out` is a
            // separate `&mut` borrow, so it cannot overlap the ring.
            unsafe {
                let src = self.ring.buf.as_ptr() as *const T;
                std::ptr::copy_nonoverlapping(src.add(start), out.as_mut_ptr(), first);
                std::ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(first), second);
            }
        }

        #[cfg(loom)]
        {
            for (idx, write_here) in out[..n_to_pop].iter_mut().enumerate() {
                *write_here = self.pop_cell(self.head.wrapping_add(idx));
            }
        }
        self.update_state_after_pop(n_to_pop);
        n_to_pop
    }

    /// Drops at most `n` of the oldest items and returns how many were dropped.
    pub fn skip(&mut self, n: usize) -> usize {
        self.drain().take(n).count()
    }

    /// Drops everything currently published and returns how many items that was.
    pub fn clear(&mut self) -> usize {
        self.drain().count()
    }

    // --------------------------------------------------------------- observers
    /// True once either end has been dropped or closed.
    ///
    /// `Acquire`: after observing `true`, every item the producer pushed before
    /// closing is visible, so `is_closed()` followed by `pop()` until `None`
    /// is a complete drain.
    pub fn is_closed(&self) -> bool {
        self.ring.closed.load(Ordering::Acquire)
    }

    /// Closes the channel: the producer's next push fails.
    pub fn close(&mut self) {
        self.ring.closed.store(true, Ordering::Release);
    }

    /// Items currently published and not yet consumed.
    pub fn len(&self) -> usize {
        self.ring
            .tail
            .load(Ordering::Relaxed)
            .wrapping_sub(self.head)
    }

    /// True when nothing is published.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the producer cannot push anything more.
    pub fn is_full(&self) -> bool {
        self.free_space() == 0
    }

    /// Free slots from the consumer's point of view.
    pub fn free_space(&self) -> usize {
        N - self.len()
    }

    /// The compile-time capacity `N`.
    pub const fn capacity(&self) -> usize {
        N
    }
}

impl<T, const N: usize> Drop for Consumer<T, N> {
    fn drop(&mut self) {
        self.ring.closed.store(true, Ordering::Release);
    }
}

/// Owning iterator returned by [`Consumer::drain`].
pub struct Drain<'a, T, const N: usize> {
    consumer: &'a mut Consumer<T, N>,
    remaining: usize,
}

impl<T, const N: usize> Iterator for Drain<'_, T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let item = self.consumer.pop_cell(self.consumer.head);
        self.consumer.update_state_after_pop(1);
        self.remaining -= 1;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<T, const N: usize> ExactSizeIterator for Drain<'_, T, N> {}

/// Transactional zero-copy view returned by [`Consumer::read_chunk`]: inspect
/// everything published, then `commit` exactly what you consumed. Dropping it
/// without a commit consumes nothing.
///
/// The view holds `&mut Consumer`, so nothing else can pop while it lives.
///
/// ```compile_fail,E0499
/// let (mut tx, mut rx) = channel::spsc::channel::<u64, 4>();
/// tx.push(1).unwrap();
/// let chunk = rx.read_chunk();
/// let stolen = rx.pop();          // E0499: `rx` is still borrowed by `chunk`
/// chunk.commit(1);
/// ```
pub struct ReadChunk<'a, T, const N: usize> {
    consumer: &'a mut Consumer<T, N>,
    len: usize,
}

impl<T, const N: usize> ReadChunk<'_, T, N> {
    /// Items in the view, fixed when the view was opened.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when nothing was published when the view was opened.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The items as two slices, oldest first: the second one is non-empty only
    /// when the view wraps around the end of the buffer.
    #[cfg(not(loom))]
    pub fn as_slices(&self) -> (&[T], &[T]) {
        let start = self.consumer.head & (N - 1);
        let first_len = self.len.min(N - start);
        let second_len = self.len - first_len;
        // SAFETY: invariant 3 for the cast; invariant 1: `start + first_len <=
        // N`, `second_len <= start`. Invariant 2: the `len` slots from `head`
        // were published before the view opened and stay owned by the consumer,
        // which this view borrows mutably, so `head` cannot move while the
        // returned slices live.
        unsafe {
            let base = self.consumer.ring.buf.as_ptr() as *const T;
            (
                std::slice::from_raw_parts(base.add(start), first_len),
                std::slice::from_raw_parts(base, second_len),
            )
        }
    }

    /// Iterates the items in the view, oldest first.
    #[cfg(not(loom))]
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let (first, second) = self.as_slices();
        first.iter().chain(second)
    }

    /// Consumes the first `n` items of the view (clamped to its length) with a
    /// single release store.
    ///
    /// Takes `self` by value: a view can only be committed once, and no slice
    /// can outlive it. `T: Copy` because the committed items are released
    /// without being dropped.
    pub fn commit(self, n: usize)
    where
        T: Copy,
    {
        let n = n.min(self.len);
        self.consumer.update_state_after_pop(n);
    }

    /// Consumes every item in the view. Same contract as [`Self::commit`].
    pub fn commit_all(self)
    where
        T: Copy,
    {
        let n = self.len;
        self.consumer.update_state_after_pop(n);
    }
}

/// Creates a bounded SPSC channel of capacity `N`, a power of two of at
/// least 2; anything else is a compile error.
pub fn channel<T, const N: usize>() -> (Producer<T, N>, Consumer<T, N>) {
    let ring = Arc::new(Ring::<T, N>::new());
    (Producer::new(Arc::clone(&ring)), Consumer::new(ring))
}

// A refactor that breaks `Send` (an `Rc` field, a raw pointer...) must fail
// here, at compile time, not in production.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<Producer<u64, 4>>();
    assert_send::<Consumer<u64, 4>>();
};

#[cfg(test)]
#[path = "tests/standard.rs"]
mod standard_test;

#[cfg(test)]
#[path = "tests/loom.rs"]
mod loom_test;
