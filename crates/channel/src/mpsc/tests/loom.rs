#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(loom)]
mod loom_tests {
    use loom::thread;

    use crate::error::Error;
    use crate::mpsc::channel;

    // Boxed payloads: any read of a slot that is unpublished, or recycled too
    // early, becomes an unsynchronised access that loom reports.

    // PUBLICATION edge: two concurrent producers; the consumer must never
    // read a slot before the Release store of its seq.
    #[test]
    fn two_producers_publish_without_tear() {
        loom::model(|| {
            let (tx, mut rx) = channel::<Box<usize>, 2>();
            let tx2 = tx.clone();

            let p1 = thread::spawn(move || tx.push(Box::new(1)).unwrap());
            let p2 = thread::spawn(move || tx2.push(Box::new(2)).unwrap());

            let mut got = Vec::with_capacity(2);
            while got.len() < 2 {
                match rx.pop() {
                    Some(v) => got.push(*v),
                    None => thread::yield_now(),
                }
            }
            p1.join().unwrap();
            p2.join().unwrap();
            got.sort_unstable();
            assert_eq!(got, [1, 2]);
        });
    }

    // RECYCLE edge: the ring is full, a pop frees a slot, the producer reuses
    // it. This is the "fungible credits" scenario: the spin on seq in
    // write_cell is the only thing preventing an overwrite of a read in progress.
    #[test]
    fn slot_reuse_at_full_boundary() {
        loom::model(|| {
            let (tx, mut rx) = channel::<Box<usize>, 2>();
            tx.push(Box::new(0)).unwrap();
            tx.push(Box::new(1)).unwrap();

            let producer = thread::spawn(move || {
                let mut item = Box::new(2);
                loop {
                    match tx.push(item) {
                        Ok(()) => break,
                        Err(Error::ChannelFull(back)) => {
                            item = back;
                            thread::yield_now();
                        }
                        Err(other) => panic!("{other:?}"),
                    }
                }
            });

            let mut expected = 0usize;
            while expected < 3 {
                match rx.pop() {
                    Some(v) => {
                        assert_eq!(*v, expected); // single producer => strict FIFO
                        expected += 1;
                    }
                    None => thread::yield_now(),
                }
            }
            producer.join().unwrap();
        });
    }

    // Credits going negative: two concurrent batches on N=2; the refunding
    // fetch_sub/fetch_add pairs must neither lose nor duplicate a credit.
    //
    // Each producer makes a single attempt and the consumer a single pop while
    // they run: retry loops on `Ok(0)` would make the model unbounded, and the
    // interleavings of interest are all reachable without them.
    #[test]
    fn concurrent_batches_never_overcommit() {
        loom::model(|| {
            let (tx, mut rx) = channel::<usize, 2>();
            let tx2 = tx.clone();
            let tx_main = tx.clone(); // to check the capacity afterwards

            let p1 = thread::spawn(move || tx.push_batch(&[10, 11]).unwrap());
            let p2 = thread::spawn(move || tx2.push_batch(&[20, 21]).unwrap());

            // A pop racing the producers can free a credit mid-flight.
            let early = rx.pop();
            let popped_early = usize::from(early.is_some());
            let mut got: Vec<usize> = early.into_iter().collect();
            let sent = p1.join().unwrap() + p2.join().unwrap();
            got.extend(rx.drain());

            // Never more than the capacity in flight, and every granted item
            // arrives exactly once.
            assert!(sent <= 2 + popped_early, "overcommit: {sent} sent");
            assert_eq!(got.len(), sent, "item lost or duplicated: {got:?}");

            // Each batch contributes a prefix of itself, in order.
            for batch in [[10, 11], [20, 21]] {
                let mine: Vec<usize> = got.iter().copied().filter(|v| batch.contains(v)).collect();
                assert_eq!(mine, batch[..mine.len()], "batch order violated: {got:?}");
            }

            // Every credit came back: the ring is empty and fully available.
            assert!(rx.is_empty());
            assert_eq!(tx_main.free_space(), 2);
        });
    }

    // drain must only yield the published prefix, never read a slot that
    // tail has reserved but its producer has not written yet.
    #[test]
    fn drain_yields_only_published_prefix() {
        loom::model(|| {
            let (tx, mut rx) = channel::<Box<usize>, 2>();

            let producer = thread::spawn(move || {
                tx.push(Box::new(0)).unwrap();
                tx.push(Box::new(1)).unwrap();
            });

            let mut got = Vec::with_capacity(2);
            while got.len() < 2 {
                let before = got.len();
                for v in rx.drain() {
                    got.push(*v);
                }
                if got.len() == before {
                    thread::yield_now();
                }
            }
            producer.join().unwrap();
            assert_eq!(got, [0, 1]);
        });
    }

    // Validates the observing spin in abandon_cell: abandoning ticket 2
    // (slot 0) races the recycle store seq[0]=2 made by the consumer when it
    // consumes ticket 0. Without the prior read, the abandon store can land
    // BEFORE the consumer's in the modification order (the Relaxed credits
    // give no happens-before): the abandon is lost and the consumer stalls,
    // which loom reports as a lack of progress.
    #[test]
    fn abandon_races_previous_lap_recycle() {
        struct Liar;
        impl Iterator for Liar {
            type Item = Box<usize>;
            fn next(&mut self) -> Option<Box<usize>> {
                None // promises 1 (size_hint), yields nothing: forces the abandon
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (1, Some(1))
            }
        }
        impl ExactSizeIterator for Liar {}

        loom::model(|| {
            let (tx, mut rx) = channel::<Box<usize>, 2>();
            tx.push(Box::new(0)).unwrap(); // ticket 0, slot 0
            tx.push(Box::new(1)).unwrap(); // ticket 1, slot 1: queue full
            let tx_main = tx.clone(); // to check the capacity afterwards

            let producer = thread::spawn(move || {
                // Waits for the credit freed by popping ticket 0. Nobody
                // else competes, so the next reservation succeeds.
                while tx.free_space() == 0 {
                    thread::yield_now();
                }
                // Reserves ticket 2 (slot 0) and abandons it at once:
                // abandon_cell spins here on seq[0] == 2, the value the
                // consumer is writing. This is the race under test.
                assert_eq!(tx.push_iter(&mut Liar).unwrap(), 0);

                // Keeps pushing behind the hole: ticket 3, slot 1.
                let mut item = Box::new(9);
                loop {
                    match tx.push(item) {
                        Ok(()) => break,
                        Err(Error::ChannelFull(back)) => {
                            item = back;
                            thread::yield_now();
                        }
                        Err(other) => panic!("{other:?}"),
                    }
                }
            });

            // The consumer must see 0, 1, skip the abandoned ticket 2, then 9.
            let mut got = Vec::with_capacity(3);
            while got.len() < 3 {
                match rx.pop() {
                    Some(v) => got.push(*v),
                    None => thread::yield_now(),
                }
            }
            producer.join().unwrap();
            assert_eq!(got, [0, 1, 9]);

            // The skip returned the abandoned ticket's credit: full capacity.
            tx_main.push(Box::new(7)).unwrap();
            tx_main.push(Box::new(8)).unwrap();
            assert!(matches!(
                tx_main.push(Box::new(99)),
                Err(Error::ChannelFull(_))
            ));
            assert_eq!(rx.pop().as_deref(), Some(&7));
            assert_eq!(rx.pop().as_deref(), Some(&8));
            assert_eq!(rx.pop(), None);
        });
    }

    #[test]
    fn residue_dropped_exactly_once() {
        use std::sync::Arc as StdArc;
        use std::sync::atomic::{AtomicUsize as StdAtomicUsize, Ordering as StdOrdering};

        struct Counted(StdArc<StdAtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, StdOrdering::SeqCst);
            }
        }

        loom::model(|| {
            let drops = StdArc::new(StdAtomicUsize::new(0));
            {
                let (tx, mut rx) = channel::<Counted, 2>();
                tx.push(Counted(StdArc::clone(&drops))).unwrap();
                tx.push(Counted(StdArc::clone(&drops))).unwrap();
                drop(rx.pop());
            }
            assert_eq!(drops.load(StdOrdering::SeqCst), 2);
        });
    }

    #[test]
    fn push_after_consumer_drop_fails() {
        loom::model(|| {
            let (tx, rx) = channel::<usize, 2>();
            drop(rx);
            assert!(matches!(tx.push(1), Err(Error::ReceiverDisconnected(_))));
        });
    }

    // A drain racing two lying push_iter calls: skip_abandoned can cross
    // the snapshot boundary (the second run of abandons is contiguous with
    // the first). remaining must saturate instead of underflowing, and every
    // credit must come back exactly once.
    #[test]
    fn drain_races_two_lying_batches() {
        struct Liar;
        impl Iterator for Liar {
            type Item = Box<usize>;
            fn next(&mut self) -> Option<Box<usize>> {
                None
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (2, Some(2))
            }
        }
        impl ExactSizeIterator for Liar {}

        loom::model(|| {
            let (tx, mut rx) = channel::<Box<usize>, 4>();
            let tx2 = tx.clone();

            // Run 1: abandons made before the drain, inside its snapshot.
            assert_eq!(tx.push_iter(&mut Liar).unwrap(), 0);

            // Run 2: abandons racing the drain, past its snapshot.
            let producer = thread::spawn(move || {
                assert_eq!(tx2.push_iter(&mut Liar).unwrap(), 0);
            });

            // remaining = 2; depending on the interleaving skip_abandoned skips 2 or 4.
            assert_eq!(rx.drain().count(), 0);
            producer.join().unwrap();

            // Clears the remaining holes, then the full capacity is back.
            assert!(rx.pop().is_none());
            for i in 0..4 {
                tx.push(Box::new(i)).unwrap();
            }
            assert!(matches!(tx.push(Box::new(9)), Err(Error::ChannelFull(_))));
            for i in 0..4 {
                assert_eq!(rx.pop().as_deref(), Some(&i));
            }
            assert_eq!(rx.pop(), None);
        });
    }
}
