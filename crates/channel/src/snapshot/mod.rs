//! Latest-value channel between one writer and one reader.
//!
//! The writer appends each new value as a node at the end of a linked list.
//! The reader checks whether its node has a successor and, if so, jumps to the
//! newest node, freeing the ones it walks past. Intermediate values can be
//! skipped: the reader only ever wants the latest one.
//!
//! # Protocol
//!
//! Each node carries its payload and a `next` pointer written at most once:
//!
//! - `next == null`: this is the tail; the writer may still append.
//! - `next` points to another node: a newer value exists, and the writer no
//!   longer references this one, so the reader owns it and frees it on
//!   `update`.
//! - `next` points to *itself*: the channel is closed. Either end can close
//!   it, and the state is terminal.
//!
//! # Memory
//!
//! The reader frees every node it walks past. The final node is shared: once
//! both ends are closed they point at the same node, and whichever end is
//! dropped second frees it. Nothing leaks once both ends are gone.
//!
//! Growth is bounded only by the reader keeping up. A reader that stops calling
//! [`Reader::update`] while the writer keeps publishing accumulates one node
//! per publish, freed only when it catches up or is dropped, and **the writer
//! has no way to observe that**. If your writer can
//! outrun its reader, put a shared depth counter in front of it.
//!
//! # Thread safety
//!
//! Both ends hand out `&T` into the node they point at, and that can be the
//! same node on two threads at once, so sending either end requires
//! `T: Send + Sync`:
//!
//! ```compile_fail,E0277
//! use std::cell::Cell;
//!
//! let (_reader, writer) = channel::snapshot::channel(Cell::new(0));
//! std::thread::spawn(move || drop(writer));
//! ```
//!
//! # Safety invariants
//!
//! Every `unsafe` block in this module relies on one or more of these.
//!
//! 1. Nodes are only freed by the reader, and only a node whose `next` points
//!    to a *different* node, with one exception: the final node (invariant 6).
//! 2. The writer's `tail` has `next` either null or pointing to itself, so by
//!    invariant 1 it is never freed while the writer exists.
//! 3. The reader's `current` is only ever freed by the reader itself, after
//!    moving past it, so it is valid for as long as the reader exists.
//! 4. Once the writer's CAS makes a node's `next` point to a newer node, the
//!    writer never dereferences that node again, so the reader may free it.
//! 5. `next` changes at most once, from null to a newer node or to the node
//!    itself, so a non-null `next` never changes again.
//! 6. Each end closes the channel before it is dropped, and closing leaves it on
//!    the node whose `next` points to itself: the writer closes its own tail,
//!    the reader walks up to that tail (or closes it first, and the writer can
//!    then no longer publish past it). Both ends therefore drop on the same
//!    final node, and its `released` flag, swapped by each end in turn, makes
//!    exactly one of them, the second, free it.

use std::ptr;

use super::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use crate::ReceiverDisconnected;

/// One published value, linked to its successor.
///
/// `data` is immutable after construction. `next` transitions at most once,
/// from null to either a newer node or to itself. `released` is only used on
/// the final node, by the two ends' `Drop` (invariant 6).
struct Snapshot<T> {
    data: T,
    next: AtomicPtr<Snapshot<T>>,
    released: AtomicBool,
}

impl<T> Snapshot<T> {
    fn new(data: T) -> *mut Self {
        Box::into_raw(Box::new(Self {
            data,
            next: AtomicPtr::new(ptr::null_mut()),
            released: AtomicBool::new(false),
        }))
    }

    /// Called by each end on drop, after closing, with the final node
    /// (invariant 6). The first caller only marks it; the second frees it.
    ///
    /// # Safety
    ///
    /// `node` must be the final node of a closed channel, and the caller must
    /// not touch it after this call.
    unsafe fn release(node: *mut Self) {
        // SAFETY: the final node is valid until the second `release` (the
        // caller's contract). `AcqRel`: the first end's last accesses happen
        // before the second end frees the node.
        if unsafe { (*node).released.swap(true, Ordering::AcqRel) } {
            // SAFETY: both ends are done with the node, and it came from
            // `Box::into_raw` in `Snapshot::new`.
            drop(unsafe { Box::from_raw(node) });
        }
    }
}

/// Publishing end of a snapshot channel.
///
/// The writer owns the logical tail of the linked list. It appends new
/// snapshots by writing to the current tail's `next` pointer, then moving its
/// own `tail` pointer to the newly published snapshot.
///
/// The writer does not own the full chain: consumed snapshots are reclaimed by
/// the reader.
pub struct Writer<T> {
    tail: *mut Snapshot<T>,
}
impl<T> Writer<T> {
    /// The last value this writer published (or the initial one).
    pub fn current(&self) -> &T {
        // SAFETY: invariant 2: `tail` is valid while the writer exists, and
        // `data` is never mutated after construction.
        unsafe { &(*self.tail).data }
    }

    /// Publishes a new immutable value.
    ///
    /// Appends a node after the current tail, then advances the tail pointer.
    /// Returns the value inside [`ReceiverDisconnected`] if the channel is
    /// closed, so a caller that wants to route it elsewhere does not lose it.
    pub fn publish(&mut self, new_data: T) -> Result<(), ReceiverDisconnected<T>> {
        let new_ptr = Snapshot::new(new_data);

        // Link the new node after the tail. The CAS can only fail if `next`
        // already points to the tail itself, meaning the channel is closed.
        // SAFETY: invariant 2: `tail` is valid.
        let linked = unsafe {
            (*self.tail).next.compare_exchange(
                ptr::null_mut(),
                new_ptr,
                Ordering::Release,
                Ordering::Relaxed,
            )
        };
        match linked {
            Ok(_) => {
                // Invariant 4: the old tail is never touched again.
                self.tail = new_ptr;
                Ok(())
            }
            Err(_) => {
                // SAFETY: the exchange failed, so `new_ptr` was never stored
                // and no other thread can have observed it. We are still its
                // only owner.
                let reclaimed = unsafe { Box::from_raw(new_ptr) };
                Err(ReceiverDisconnected::with_value(reclaimed.data))
            }
        }
    }

    /// Closes the channel. The reader still sees every value published before.
    pub fn close(&mut self) {
        // SAFETY: invariant 2: `tail` is valid. Storing the self-pointer is the
        // only transition left for a tail (invariant 5), and the reader may
        // race it with the same store.
        unsafe { (*self.tail).next.store(self.tail, Ordering::Release) };
    }

    /// True once either end has closed the channel.
    pub fn is_closed(&self) -> bool {
        // SAFETY: invariant 2: `tail` is valid.
        self.tail == unsafe { (*self.tail).next.load(Ordering::Acquire) }
    }
}
impl<T> Drop for Writer<T> {
    fn drop(&mut self) {
        self.close();
        // SAFETY: after `close`, `tail` is the final node (invariant 6), and
        // the writer never touches it again.
        unsafe { Snapshot::release(self.tail) };
    }
}

/// Reading end of a snapshot channel.
///
/// The reader points to its currently visible snapshot. It may advance to the
/// newest snapshot when the writer has published one, freeing the ones it
/// walks past.
pub struct Reader<T> {
    current: *mut Snapshot<T>,
}

impl<T> Reader<T> {
    /// The value the reader currently sees. Never fails, never blocks.
    pub fn current(&self) -> &T {
        // SAFETY: invariant 3: `current` is valid while the reader exists, and
        // `data` is never mutated after construction.
        unsafe { &(*self.current).data }
    }

    /// Advances the reader to the newest snapshot, if one has been published.
    ///
    /// `Ok(false)` means the current snapshot is still the newest. The error is
    /// only returned once the reader has reached the final, closed snapshot, so
    /// everything published before the close is still seen first.
    pub fn update(&mut self) -> Result<bool, ReceiverDisconnected> {
        // SAFETY: invariant 3: `current` is valid.
        let mut next = unsafe { (*self.current).next.load(Ordering::Acquire) };

        if next.is_null() {
            return Ok(false);
        }
        if self.is_closed() {
            return Err(ReceiverDisconnected::default());
        }

        // Walk to the last node, freeing the ones left behind.
        while !next.is_null() {
            if next == self.current {
                break;
            }
            let old = self.current;
            self.current = next;
            // SAFETY: `old.next` points to a different node, so by invariants
            // 1 and 4 the writer no longer references it and it is ours to
            // free. It came from `Box::into_raw` in `Snapshot::new`.
            unsafe { drop(Box::from_raw(old)) };
            // SAFETY: `next` was published with `Release` and loaded with
            // `Acquire`, so the node is fully initialised; invariant 3 keeps it
            // valid from now on.
            next = unsafe { (*self.current).next.load(Ordering::Acquire) };
        }

        Ok(true)
    }

    /// Closes the channel: the writer's next publish fails.
    pub fn close(&mut self) {
        loop {
            // SAFETY: invariant 3: `current` is valid.
            let closed = unsafe {
                (*self.current).next.compare_exchange(
                    ptr::null_mut(),
                    self.current,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
            };
            if closed.is_ok() || self.is_closed() {
                return;
            }
            // A newer node exists: move to it and try again on the new tail.
            if self.update().is_err() {
                return;
            }
        }
    }

    /// True once either end has closed the channel and the reader has reached
    /// the final snapshot.
    pub fn is_closed(&self) -> bool {
        // SAFETY: invariant 3: `current` is valid.
        self.current == unsafe { (*self.current).next.load(Ordering::Acquire) }
    }
}

impl<T> Drop for Reader<T> {
    fn drop(&mut self) {
        self.close();
        // SAFETY: after `close`, `current` is the final node (invariant 6),
        // and the reader never touches it again.
        unsafe { Snapshot::release(self.current) };
    }
}

/// Creates a single-writer, single-reader snapshot channel seeded with `init`.
///
/// - [`Writer`] appends new snapshots;
/// - [`Reader`] observes and advances through them, freeing consumed ones;
/// - the final snapshot is freed by whichever end is dropped last.
///
/// The seed is immediately readable: [`Reader::current`] never fails and never
/// blocks, which is the point: a consumer of routing state has a valid table
/// from its first instruction.
pub fn channel<T>(init: T) -> (Reader<T>, Writer<T>) {
    let ptr = Snapshot::new(init);
    (Reader { current: ptr }, Writer { tail: ptr })
}

// SAFETY: sending the reader moves the right to read and free nodes to another
// thread; the values it frees were created on the writer's thread (`T: Send`),
// and it may read a node the writer reads at the same time (`T: Sync`).
unsafe impl<T: Send + Sync> Send for Reader<T> {}
// SAFETY: same reasoning as for `Reader`: the writer moves values to the
// reader's thread and may share a node with it.
unsafe impl<T: Send + Sync> Send for Writer<T> {}

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<Reader<u64>>();
    assert_send::<Writer<u64>>();
};

#[cfg(test)]
#[path = "tests/standard.rs"]
mod standard_test;

#[cfg(test)]
#[path = "tests/loom.rs"]
mod loom_test;
