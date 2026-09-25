//! # Safety invariants
//!
//! Every `unsafe` block in this file relies on one or more of these.
//!
//! 1. **Bounds.** Every buffer access goes through `idx & (N - 1)`. `N` is a
//!    power of two (checked at compile time in `Ring::new`), so this equals
//!    `idx % N`, which is `< N == buff.len()`.
//! 2. **Slot ownership is carried by `seq`.** Ticket `t` maps to cell
//!    `t & (N - 1)`, and tickets are unique because they come from a
//!    `fetch_add` on `tail`. For that cell:
//!    - `seq == t`: empty, owned by the producer holding ticket `t`;
//!    - `seq == t + 1`: holds the value of ticket `t`, owned by the consumer;
//!    - `seq == t + N`: empty again (consumed or abandoned), handed over to
//!      ticket `t + N`.
//!
//!    Each transition is a `Release` store made by the current owner after it
//!    is done with `data`, and the next owner loads `seq` with `Acquire` before
//!    touching `data`. Every write to `data` therefore happens-before the read
//!    that consumes it, and every read happens-before the next write.
//! 3. **Exclusive access at drop time.** `Ring::drop` runs with `&mut self`,
//!    after every handle is gone, so a cell whose `seq` still says "published"
//!    holds a value nobody will ever read.

use std::mem::MaybeUninit;

use crate::{error::*, util::CachePadded};

use super::{
    MAX_SPIN_BEFORE_FAIL,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering, fence},
        cell::UnsafeCell,
    },
};

struct Cell<T> {
    data: UnsafeCell<MaybeUninit<T>>,
    seq: AtomicUsize,
}

impl<T> Cell<T> {
    fn new(seq: usize) -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
            seq: AtomicUsize::from(seq),
        }
    }
}

struct Ring<T, const N: usize> {
    buff: Box<[Cell<T>; N]>,
    tail: CachePadded<AtomicUsize>,
    tickets: CachePadded<AtomicI64>,

    closed: AtomicBool,
    producers_count: AtomicU32,
}

// SAFETY: the ring only moves `T` values from producer threads to the consumer
// thread, so `T: Send` is the only requirement. Concurrent access to `data` is
// serialised by invariant 2, and no `&T` is ever handed to two threads.
unsafe impl<T: Send, const N: usize> Send for Ring<T, N> {}
// SAFETY: see the `Send` impl above.
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
        const { assert!(N < i64::MAX as usize, "N must fit the i64 credit counter") };

        // `Box::new(std::array::from_fn(..))` would build the array on the
        // stack first and then move it, which overflows the stack for a large
        // `N`. Building it in place on the heap avoids the copy.
        let mut heap_box: Box<[MaybeUninit<Cell<T>>]> = Box::new_uninit_slice(N);
        for (idx, slot) in heap_box.iter_mut().enumerate() {
            slot.write(Cell::new(idx));
        }
        // SAFETY: the loop above initialised all `N` elements.
        let buff: Box<[Cell<T>; N]> = unsafe { heap_box.assume_init() }
            .try_into()
            .unwrap_or_else(|_| unreachable!("boxed slice always has length N"));

        Self {
            buff,
            tail: CachePadded::new(AtomicUsize::from(0)),
            tickets: CachePadded::new(AtomicI64::from(N as i64)),

            closed: AtomicBool::new(false),
            producers_count: AtomicU32::new(0),
        }
    }

    fn add_producer(&self) {
        self.producers_count.fetch_add(1, Ordering::Relaxed);
    }

    fn drop_producer(&self) {
        let prev = self.producers_count.fetch_sub(1, Ordering::Release);
        debug_assert!(prev != 0, "producer count underflow");
        if prev == 1 {
            fence(Ordering::Acquire);
            self.closed.store(true, Ordering::Release);
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl<T, const N: usize> Drop for Ring<T, N> {
    fn drop(&mut self) {
        // The consumer's head is not stored in the ring, so scan the last `N`
        // tickets and drop every slot still in the "published" state.
        let tail = self.tail.load(Ordering::Relaxed);
        let range = (0..N).map(|i| tail.wrapping_sub(N).wrapping_add(i));
        for idx in range {
            // SAFETY: invariant 1.
            let cell = unsafe { self.buff.get_unchecked(idx & (N - 1)) };
            if cell.seq.load(Ordering::Relaxed) == idx.wrapping_add(1) {
                // SAFETY: invariants 2 and 3: `seq == idx + 1` means the slot
                // holds the unread value of ticket `idx`, and nothing else can
                // touch it any more.
                unsafe { cell.data.with_mut(|ptr| (*ptr).assume_init_drop()) }
            }
        }
    }
}

/// Sending half of an MPSC channel. Clone it to add producers.
pub struct Producer<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
}
impl<T, const N: usize> std::fmt::Debug for Producer<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("mpsc::Producer")
            .field("capacity", &N)
            .field("len", &self.len())
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl<T, const N: usize> Producer<T, N> {
    // ------------------------------------------------------ protocol internals
    fn new(ring: Arc<Ring<T, N>>) -> Self {
        ring.add_producer();
        Self { ring }
    }

    fn write_cell(&self, idx: usize, value: T) {
        // SAFETY: invariant 1.
        let cell = unsafe { self.ring.buff.get_unchecked(idx & (N - 1)) };
        // The credit taken before the ticket guarantees the slot is free or
        // about to be: the consumer returns credits only after recycling a
        // slot. What may lag is the visibility of that recycle, hence the spin.
        while cell.seq.load(Ordering::Acquire) != idx {
            #[cfg(loom)]
            loom::thread::yield_now();
            #[cfg(not(loom))]
            std::hint::spin_loop();
        }

        // SAFETY: invariant 2: `seq == idx` and ticket `idx` is ours, so the
        // slot is empty and nobody else touches `data` until the store below.
        unsafe {
            cell.data.with_mut(|ptr| (*ptr).write(value));
        }
        cell.seq.store(idx.wrapping_add(1), Ordering::Release);
    }

    fn abandon_cell(&self, idx: usize) {
        // SAFETY: invariant 1.
        let cell = unsafe { self.ring.buff.get_unchecked(idx & (N - 1)) };
        // The recycled value must be *observed* before it is overwritten.
        // Without this read, the store below could land before the consumer's
        // recycle store in the modification order (the relaxed credits give no
        // happens-before edge) and be lost. Same near-zero wait as `write_cell`.
        while cell.seq.load(Ordering::Acquire) != idx {
            #[cfg(loom)]
            loom::thread::yield_now();
            #[cfg(not(loom))]
            std::hint::spin_loop();
        }
        // Straight to the recycled value: the consumer skips this slot and
        // returns its credit in FIFO order. No data is ever written here.
        cell.seq.store(idx.wrapping_add(N), Ordering::Release);
    }

    fn try_reserve(&self) -> bool {
        // A plain load first: when the ring is full this avoids a
        // read-modify-write that would only have to be undone.
        if self.ring.tickets.load(Ordering::Relaxed) <= 0 {
            return false;
        }
        if self.ring.tickets.fetch_sub(1, Ordering::Relaxed) <= 0 {
            self.ring.tickets.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    // ----------------------------------------------------------------- pushing
    /// Pushes one value at the tail of the global FIFO.
    ///
    /// Fail-fast and lock-free: returns [`Error::ChannelFull`] (value handed
    /// back) when no credit is free, which includes producers racing for the
    /// last slots, and [`Error::ReceiverDisconnected`] once the consumer is
    /// gone. Never blocks on a full queue.
    pub fn push(&self, value: T) -> Result<(), Error<T>> {
        if self.is_closed() {
            return Err(Error::disconnected(value));
        }

        if !self.try_reserve() {
            return Err(Error::ChannelFull(value));
        }

        let pos = self.ring.tail.fetch_add(1, Ordering::Relaxed);
        self.write_cell(pos, value);
        Ok(())
    }

    /// Pushes up to `iter.len()` items as one contiguous run of tickets: no
    /// item from another producer can interleave inside the run. Takes what
    /// fits and returns how many items were consumed from the iterator; the
    /// iterator resumes right after the last pushed item.
    ///
    /// If the iterator yields fewer items than `len()` promised (a broken
    /// `ExactSizeIterator`), the remaining reserved slots are abandoned and
    /// silently skipped by the consumer, which recovers their capacity: the
    /// run is simply shorter. No panic, no stall.
    pub fn push_iter(
        &self,
        mut iter: impl ExactSizeIterator<Item = T>,
    ) -> Result<usize, ReceiverDisconnected<()>> {
        if self.is_closed() {
            return Err(ReceiverDisconnected::default());
        }

        let wanted = iter.len().min(N);
        if wanted == 0 {
            return Ok(0);
        }
        let wanted = wanted as i64;

        let tickets = self.ring.tickets.fetch_sub(wanted, Ordering::Relaxed);
        let granted = tickets.clamp(0, wanted);
        if granted < wanted {
            self.ring
                .tickets
                .fetch_add(wanted - granted, Ordering::Relaxed);
        }
        if granted == 0 {
            return Ok(0);
        }

        let granted = granted as usize;
        let pos_init = self.ring.tail.fetch_add(granted, Ordering::Relaxed);
        let mut run = ReservedRun {
            producer: self,
            pos_init,
            granted,
            filled: 0,
        };
        while run.filled < granted {
            match iter.next() {
                Some(value) => {
                    self.write_cell(pos_init.wrapping_add(run.filled), value);
                    run.filled += 1;
                }
                None => break,
            };
        }
        let pushed = run.filled;
        drop(run);
        Ok(pushed)
    }

    /// [`Self::push_iter`] over a slice. A slice length cannot lie, which makes
    /// this the preferred batch entry point. `T: Copy` because items are copied
    /// out of the borrowed slice.
    pub fn push_batch(&self, batch: &[T]) -> Result<usize, ReceiverDisconnected<()>>
    where
        T: Copy,
    {
        self.push_iter(&mut batch.iter().copied())
    }

    // --------------------------------------------------------------- observers
    /// Best effort: true when no credit was free at the instant of the load.
    pub fn is_full(&self) -> bool {
        self.ring.tickets.load(Ordering::Relaxed) <= 0
    }

    /// True once the consumer is gone or someone called `close`.
    pub fn is_closed(&self) -> bool {
        self.ring.closed.load(Ordering::Relaxed)
    }

    /// Approximate occupancy (capacity minus free credits); counts in-flight
    /// reservations as occupied.
    pub fn len(&self) -> usize {
        N - self.ring.tickets.load(Ordering::Relaxed).max(0) as usize
    }

    /// Best effort: true when every credit is free (no items, no reservations).
    pub fn is_empty(&self) -> bool {
        self.ring.tickets.load(Ordering::Relaxed) == N as i64
    }

    /// Approximate number of free slots at the instant of the load.
    pub fn free_space(&self) -> usize {
        self.ring.tickets.load(Ordering::Relaxed).max(0) as usize
    }

    /// The compile-time capacity `N`.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Closes the channel for every handle. Items already pushed stay
    /// readable by the consumer.
    pub fn close(&self) {
        self.ring.close();
    }
}
impl<T, const N: usize> Clone for Producer<T, N> {
    fn clone(&self) -> Self {
        self.ring.add_producer();
        Self {
            ring: Arc::clone(&self.ring),
        }
    }
}
impl<T, const N: usize> Drop for Producer<T, N> {
    fn drop(&mut self) {
        self.ring.drop_producer();
    }
}

/// Guard over the tickets reserved by `push_iter`.
///
/// If the iterator panics, or yields fewer items than it promised, the drop
/// abandons every reserved slot that was not filled. Other producers and the
/// consumer keep working, and the capacity comes back.
struct ReservedRun<'a, T, const N: usize> {
    producer: &'a Producer<T, N>,
    pos_init: usize,
    granted: usize,
    filled: usize,
}
impl<T, const N: usize> Drop for ReservedRun<'_, T, N> {
    fn drop(&mut self) {
        for k in self.filled..self.granted {
            self.producer.abandon_cell(self.pos_init.wrapping_add(k));
        }
    }
}

/// Result of [`Consumer::try_pop`].
#[derive(Debug)]
#[must_use]
pub enum PopOutcome<T> {
    /// The head item.
    Value(T),
    /// The head slot is reserved but its producer has not published yet
    /// (mid-write). Items may exist behind it; retry shortly.
    /// Persisting `Busy` while `len() > 0` signals a stalled producer.
    Busy,
    /// Nothing has been pushed.
    Empty,
}

enum HeadState {
    Published,
    Abandoned,
    Pending,
}

/// Receiving half of an MPSC channel. There is exactly one.
pub struct Consumer<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
    head: usize,
    cached_tail: usize,
}
impl<T, const N: usize> std::fmt::Debug for Consumer<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("mpsc::Consumer")
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

    fn head_cell(&self) -> &Cell<T> {
        // SAFETY: invariant 1.
        unsafe { self.ring.buff.get_unchecked(self.head & (N - 1)) }
    }

    fn head_state(&self) -> HeadState {
        let cell = self.head_cell();
        let published = self.head.wrapping_add(1);
        let abandoned = self.head.wrapping_add(N);
        let mut iters = 0;
        loop {
            let seq = cell.seq.load(Ordering::Acquire);
            if seq == published {
                return HeadState::Published;
            } else if seq == abandoned {
                return HeadState::Abandoned;
            } else if iters > MAX_SPIN_BEFORE_FAIL {
                return HeadState::Pending;
            }
            iters += 1;
            #[cfg(loom)]
            loom::thread::yield_now();
            #[cfg(not(loom))]
            std::hint::spin_loop();
        }
    }

    fn skip_abandoned(&mut self) {
        // The slot is already in the recycled state (the producer put it
        // there); only advance past it and give its credit back, in FIFO
        // order, which is what keeps the producer spin bounded.
        while matches!(self.head_state(), HeadState::Abandoned) {
            self.head = self.head.wrapping_add(1);
            self.ring.tickets.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Only called after `head_state` returned `Published`.
    fn pop_cell(&self) -> T {
        // SAFETY: invariant 2: `seq == head + 1` was observed with `Acquire`,
        // so the slot holds the value of ticket `head` and belongs to the
        // consumer. It is read once; `update_state_after_pop` then recycles it.
        unsafe { self.head_cell().data.with(|c| (*c).assume_init_read()) }
    }

    fn update_state_after_pop(&mut self) {
        self.head_cell()
            .seq
            .store(self.head.wrapping_add(N), Ordering::Release);
        self.head = self.head.wrapping_add(1);
        self.ring.tickets.fetch_add(1, Ordering::Relaxed);
    }

    fn check_if_empty(&mut self) -> bool {
        if self.head == self.cached_tail {
            // Empty according to the cached tail: reload it and check again.
            self.cached_tail = self.ring.tail.load(Ordering::Relaxed);
            self.head == self.cached_tail
        } else {
            false
        }
    }

    fn pop_inner(&mut self) -> PopOutcome<T> {
        loop {
            if self.check_if_empty() {
                return PopOutcome::Empty;
            }
            match self.head_state() {
                HeadState::Pending => return PopOutcome::Busy,
                // Skip the hole, then look at the new head.
                HeadState::Abandoned => self.skip_abandoned(),
                HeadState::Published => {
                    let value = self.pop_cell();
                    self.update_state_after_pop();
                    return PopOutcome::Value(value);
                }
            }
        }
    }

    // ----------------------------------------------------- non-consuming reads
    /// Borrows the head item without consuming it. `None` when the queue is
    /// empty or the head producer has not published yet.
    pub fn peek(&mut self) -> Option<&T> {
        loop {
            if self.check_if_empty() {
                return None;
            }
            match self.head_state() {
                HeadState::Pending => return None,
                HeadState::Abandoned => self.skip_abandoned(),
                HeadState::Published => {
                    // SAFETY: invariant 2, as in `pop_cell`. The reference
                    // borrows `&mut self`, so the slot cannot be recycled
                    // while it lives.
                    return Some(unsafe {
                        self.head_cell().data.with(|ptr| (*ptr).assume_init_ref())
                    });
                }
            }
        }
    }

    // --------------------------------------------------------- consuming reads
    /// Consumes the head item. `None` conflates "empty" and "head not yet
    /// published"; use [`Self::try_pop`] when the distinction matters.
    pub fn pop(&mut self) -> Option<T> {
        match self.pop_inner() {
            PopOutcome::Value(t) => Some(t),
            PopOutcome::Empty | PopOutcome::Busy => None,
        }
    }

    /// Non-blocking pop with the full three-state outcome, the only way to
    /// tell [`PopOutcome::Busy`] from [`PopOutcome::Empty`].
    pub fn try_pop(&mut self) -> PopOutcome<T> {
        self.pop_inner()
    }

    /// Iterator over the published prefix, bounded to the snapshot of `tail`
    /// taken now; stops early at the first slot still being written.
    #[must_use = "the iterator is lazy: dropping it without iterating consumes nothing"]
    pub fn drain(&mut self) -> Drain<'_, T, N> {
        self.cached_tail = self.ring.tail.load(Ordering::Relaxed);
        let remaining = self.cached_tail.wrapping_sub(self.head);
        Drain {
            consumer: self,
            remaining,
        }
    }

    /// Drops at most `n` head items; returns how many were dropped.
    pub fn skip(&mut self, n: usize) -> usize {
        self.drain().take(n).count()
    }

    /// Drops everything currently drainable; returns the count.
    pub fn clear(&mut self) -> usize {
        self.drain().count()
    }

    // --------------------------------------------------------------- observers
    /// True once every producer has been dropped, or [`Consumer::close`] or
    /// [`Producer::close`] was called.
    ///
    /// `Acquire`: after observing `true`, everything the producers pushed is
    /// drainable. Check this *before* draining (see the module docs,
    /// "Shutdown protocol").
    pub fn is_closed(&self) -> bool {
        self.ring.closed.load(Ordering::Acquire)
    }

    /// Closes the channel: every producer's next push fails.
    pub fn close(&self) {
        self.ring.closed.store(true, Ordering::Release);
    }

    /// Tickets ahead of the consumer, including reserved-but-unpublished ones.
    pub fn len(&self) -> usize {
        self.ring
            .tail
            .load(Ordering::Relaxed)
            .wrapping_sub(self.head)
    }

    /// True when no ticket is ahead of the consumer.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Slots not taken by a ticket ahead of the consumer.
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

/// Draining iterator returned by [`Consumer::drain`]: each item is moved out
/// and its slot recycled immediately (credit returned per item).
pub struct Drain<'a, T, const N: usize> {
    consumer: &'a mut Consumer<T, N>,
    remaining: usize,
}

impl<T, const N: usize> Iterator for Drain<'_, T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        // A loop rather than recursion: a run of abandoned slots is skipped in
        // place.
        loop {
            if self.remaining == 0 {
                return None;
            }
            match self.consumer.head_state() {
                HeadState::Pending => return None,
                HeadState::Abandoned => {
                    let init_pos = self.consumer.head;
                    self.consumer.skip_abandoned();
                    let skipped = self.consumer.head.wrapping_sub(init_pos);
                    self.remaining = self.remaining.saturating_sub(skipped);
                }
                HeadState::Published => {
                    let item = self.consumer.pop_cell();
                    self.consumer.update_state_after_pop();
                    self.remaining -= 1;
                    return Some(item);
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.remaining))
    }
}

/// Creates a bounded MPSC channel of capacity `N`, a power of two of at least
/// 2 (anything else is a compile error). Returns `(Producer, Consumer)`; clone
/// the producer for more threads.
pub fn channel<T, const N: usize>() -> (Producer<T, N>, Consumer<T, N>) {
    let ring = Arc::new(Ring::<T, N>::new());
    (Producer::new(Arc::clone(&ring)), Consumer::new(ring))
}

// A refactor that breaks Send/Sync (an Rc field, a raw pointer...) must fail
// here, at compile time, not in production. Sync on Producer matters: a
// global handle (e.g. a logger behind a OnceLock) is shared by reference
// across threads.
const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}
    assert_send::<Producer<u64, 4>>();
    assert_sync::<Producer<u64, 4>>();
    assert_send::<Consumer<u64, 4>>();
    assert_sync::<Consumer<u64, 4>>();
};

#[cfg(test)]
#[path = "tests/standard.rs"]
mod standard_test;

#[cfg(test)]
#[path = "tests/loom.rs"]
mod loom_test;
