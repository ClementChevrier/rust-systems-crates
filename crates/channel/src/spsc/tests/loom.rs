#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(loom)]
mod loom_tests {
    use loom::thread;

    use crate::{error::Error, spsc::channel};

    #[test]
    fn publishes_two_items_in_order() {
        loom::model(|| {
            let (mut tx, mut rx) = channel::<usize, 2>();

            let producer = thread::spawn(move || {
                tx.push(0).unwrap();
                tx.push(1).unwrap();
            });

            let mut received = Vec::with_capacity(2);
            while received.len() < 2 {
                match rx.pop() {
                    Some(v) => received.push(v),
                    None => thread::yield_now(),
                }
            }
            producer.join().unwrap();
            assert_eq!(received, [0, 1]);
        });
    }

    // The critical edge: reusing a slot at the full/empty boundary. The boxed
    // payload makes loom report any read of an unpublished slot.
    #[test]
    fn slot_reuse_at_full_boundary() {
        loom::model(|| {
            let (mut tx, mut rx) = channel::<Box<usize>, 2>();
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
                        assert_eq!(*v, expected);
                        expected += 1;
                    }
                    None => thread::yield_now(),
                }
            }
            producer.join().unwrap();
        });
    }

    #[test]
    fn batch_wraps_while_consumer_pops() {
        loom::model(|| {
            let (mut tx, mut rx) = channel::<usize, 2>();
            tx.push(9).unwrap();
            assert_eq!(rx.pop(), Some(9));
            // head = tail = 1: the batch [1, 2] straddles the wrap.

            let producer = thread::spawn(move || {
                let batch = [1usize, 2];
                let mut sent = 0;
                while sent < batch.len() {
                    match tx.push_batch(&batch[sent..]) {
                        Ok(0) => thread::yield_now(),
                        Ok(pushed) => sent += pushed,
                        Err(e) => panic!("{e:?}"),
                    }
                }
            });

            let mut got = Vec::with_capacity(2);
            let mut buf = [0usize; 2];
            while got.len() < 2 {
                let n = rx.pop_batch(&mut buf);
                if n == 0 {
                    thread::yield_now();
                }
                got.extend_from_slice(&buf[..n]);
            }
            producer.join().unwrap();
            assert_eq!(got, [1, 2]);
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
                let (mut tx, mut rx) = channel::<Counted, 2>();
                tx.push(Counted(StdArc::clone(&drops))).unwrap();
                tx.push(Counted(StdArc::clone(&drops))).unwrap();
                drop(rx.pop());
            }
            assert_eq!(drops.load(StdOrdering::SeqCst), 2);
        });
    }

    // The closing protocol: observing `closed` (Acquire) implies that
    // everything pushed before the producer was dropped can be drained.
    #[test]
    fn close_is_observed_with_all_data() {
        loom::model(|| {
            let (mut tx, mut rx) = channel::<usize, 2>();

            let producer = thread::spawn(move || {
                tx.push(42).unwrap();
            });

            while !rx.is_closed() {
                thread::yield_now();
            }
            assert_eq!(rx.pop(), Some(42));
            assert_eq!(rx.pop(), None);
            producer.join().unwrap();
        });
    }

    #[test]
    fn drain_observes_published_items() {
        loom::model(|| {
            let (mut tx, mut rx) = channel::<Box<usize>, 2>();

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

    #[test]
    fn push_after_consumer_drop_fails() {
        loom::model(|| {
            let (mut tx, rx) = channel::<usize, 2>();
            drop(rx);
            assert!(matches!(tx.push(1), Err(Error::ReceiverDisconnected(_))));
        });
    }

    // peek reads the slot without advancing head: loom checks that this read
    // never precedes the publication (with a boxed payload, any early read is
    // an unsynchronised access and gets reported).
    #[test]
    fn peek_sees_published_value_without_consuming() {
        loom::model(|| {
            let (mut tx, mut rx) = channel::<Box<usize>, 2>();

            let producer = thread::spawn(move || {
                tx.push(Box::new(7)).unwrap();
            });

            loop {
                match rx.peek() {
                    Some(v) => {
                        assert_eq!(**v, 7);
                        break;
                    }
                    None => thread::yield_now(),
                }
            }
            assert_eq!(rx.len(), 1);
            assert_eq!(*rx.pop().unwrap(), 7);
            assert!(rx.pop().is_none());
            producer.join().unwrap();
        });
    }
}
