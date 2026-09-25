#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(not(loom))]
mod std_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::error::Error;
    use crate::mpsc::{PopOutcome, channel};

    const STRESS_MESSAGES: u64 = 1_000_000;
    const MIRI_STRESS_MESSAGES: u64 = 1_000;

    fn stress_messages() -> u64 {
        if cfg!(miri) {
            MIRI_STRESS_MESSAGES
        } else {
            STRESS_MESSAGES
        }
    }

    // -------------------------------------------------------- single thread

    #[test]
    fn fifo_roundtrip_with_wraparound() {
        let (tx, mut rx) = channel::<u64, 4>();
        let rounds: u64 = if cfg!(miri) { 64 } else { 4096 };
        for round in 0..rounds {
            assert!(tx.push(round).is_ok());
            assert_eq!(rx.pop(), Some(round));
        }
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn full_returns_value_then_drains_in_order() {
        let (tx, mut rx) = channel::<u64, 4>();
        assert_eq!(rx.pop(), None);
        for i in 0..4 {
            assert!(tx.push(i).is_ok());
        }
        assert!(tx.is_full());
        match tx.push(99) {
            Err(Error::ChannelFull(v)) => assert_eq!(v, 99),
            other => panic!("expected ChannelFull, got {other:?}"),
        }
        for i in 0..4 {
            assert_eq!(rx.pop(), Some(i));
        }
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn try_pop_reports_empty_and_value() {
        let (tx, mut rx) = channel::<u64, 4>();
        assert!(matches!(rx.try_pop(), PopOutcome::Empty));
        tx.push(5).unwrap();
        assert!(matches!(rx.try_pop(), PopOutcome::Value(5)));
        assert!(matches!(rx.try_pop(), PopOutcome::Empty));
    }

    // ------------------------------------------------------------- batches

    #[test]
    fn push_batch_fills_free_space_from_empty() {
        let (tx, mut rx) = channel::<u64, 4>();
        assert_eq!(tx.push_batch(&[1, 2, 3, 4, 5, 6]).unwrap(), 4);
        // Same convention as the SPSC: full => Ok(0), not an error.
        assert_eq!(tx.push_batch(&[7]).unwrap(), 0);
        for i in 1..=4 {
            assert_eq!(rx.pop(), Some(i));
        }
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn push_batch_partial_when_nearly_full() {
        let (tx, _rx) = channel::<u64, 4>();
        assert!(tx.push(0).is_ok());
        assert_eq!(tx.push_batch(&[1, 2, 3, 4, 5]).unwrap(), 3);
        assert!(tx.is_full());
    }

    #[test]
    fn push_iter_moves_non_copy_and_resumes() {
        let (tx, mut rx) = channel::<String, 4>();
        assert!(tx.push("x".to_string()).is_ok());

        let mut source = (0..10).map(|i| i.to_string());
        assert_eq!(tx.push_iter(&mut source).unwrap(), 3);
        assert_eq!(source.next().as_deref(), Some("3")); // resumes right after

        assert_eq!(rx.pop().as_deref(), Some("x"));
        assert_eq!(rx.pop().as_deref(), Some("0"));
        assert_eq!(rx.pop().as_deref(), Some("1"));
        assert_eq!(rx.pop().as_deref(), Some("2"));
        assert_eq!(rx.pop(), None);
    }

    // ------------------------------------------------------- many producers

    #[test]
    fn multi_producer_conservation_and_per_producer_fifo() {
        const PRODUCERS: u64 = 4;
        const TAG: u64 = 1_000_000_000;
        let per_producer = (stress_messages() / 16).max(1_000) / PRODUCERS;
        let (tx, mut rx) = channel::<u64, 128>();

        let handles: Vec<_> = (0..PRODUCERS)
            .map(|p| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for i in 0..per_producer {
                        let mut v = p * TAG + i;
                        loop {
                            match tx.push(v) {
                                Ok(()) => break,
                                Err(Error::ChannelFull(back)) => {
                                    v = back;
                                    std::thread::yield_now();
                                }
                                Err(e) => panic!("{e:?}"),
                            }
                        }
                    }
                })
            })
            .collect();
        drop(tx);

        let total = PRODUCERS * per_producer;
        let mut last_seen = [None::<u64>; PRODUCERS as usize];
        let mut received = 0u64;
        while received < total {
            match rx.pop() {
                Some(v) => {
                    let (p, i) = ((v / TAG) as usize, v % TAG);
                    if let Some(prev) = last_seen[p] {
                        assert!(i > prev, "per-producer FIFO violated: {prev} then {i}");
                    }
                    last_seen[p] = Some(i);
                    received += 1;
                }
                None => std::thread::yield_now(),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(rx.pop(), None);
        for (p, last) in last_seen.iter().enumerate() {
            assert_eq!(*last, Some(per_producer - 1), "producer {p} incomplete");
        }
    }

    #[test]
    fn multi_producer_batch_conservation() {
        const PRODUCERS: u64 = 2;
        const TAG: u64 = 1_000_000_000;
        const CHUNK: usize = 32;
        let per_producer = (stress_messages() / 32).max(1_000) / PRODUCERS;
        let (tx, mut rx) = channel::<u64, 128>();

        let handles: Vec<_> = (0..PRODUCERS)
            .map(|p| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let mut chunk = [0u64; CHUNK];
                    let mut next = 0u64;
                    while next < per_producer {
                        let want = ((per_producer - next) as usize).min(CHUNK);
                        for (i, c) in chunk[..want].iter_mut().enumerate() {
                            *c = p * TAG + next + i as u64;
                        }
                        let mut sent = 0;
                        while sent < want {
                            match tx.push_batch(&chunk[sent..want]) {
                                Ok(0) => std::thread::yield_now(),
                                Ok(pushed) => sent += pushed,
                                Err(e) => panic!("{e:?}"),
                            }
                        }
                        next += want as u64;
                    }
                })
            })
            .collect();
        drop(tx);

        let total = PRODUCERS * per_producer;
        let mut last_seen = [None::<u64>; PRODUCERS as usize];
        let mut received = 0u64;
        while received < total {
            match rx.pop() {
                Some(v) => {
                    let (p, i) = ((v / TAG) as usize, v % TAG);
                    if let Some(prev) = last_seen[p] {
                        assert!(i > prev, "per-producer FIFO violated inside a batch");
                    }
                    last_seen[p] = Some(i);
                    received += 1;
                }
                None => std::thread::yield_now(),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    // ------------------------------------------------------ drop & residue

    struct Counted(Arc<AtomicUsize>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drop_with_fewer_items_than_capacity() {
        // Catches a `tail - N` underflow in Drop: tail(2) < N(8).
        let drops = Arc::new(AtomicUsize::new(0));
        {
            let (tx, rx) = channel::<Counted, 8>();
            tx.push(Counted(Arc::clone(&drops))).unwrap();
            tx.push(Counted(Arc::clone(&drops))).unwrap();
            drop((tx, rx));
        }
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn drops_each_element_exactly_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        {
            let (tx, mut rx) = channel::<Counted, 8>();
            for _ in 0..6 {
                assert!(tx.push(Counted(Arc::clone(&drops))).is_ok());
            }
            drop(rx.pop());
            drop(rx.pop());
            assert_eq!(drops.load(Ordering::SeqCst), 2);
            for _ in 0..3 {
                assert!(tx.push(Counted(Arc::clone(&drops))).is_ok());
            }
        }
        // 2 consumed + 7 residues dropped by Drop for Ring.
        assert_eq!(drops.load(Ordering::SeqCst), 9);
    }

    #[test]
    fn skip_and_clear_drop_each_skipped_element() {
        let drops = Arc::new(AtomicUsize::new(0));
        let (tx, mut rx) = channel::<Counted, 8>();
        for _ in 0..5 {
            assert!(tx.push(Counted(Arc::clone(&drops))).is_ok());
        }
        assert_eq!(rx.skip(2), 2);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(rx.clear(), 3);
        assert_eq!(drops.load(Ordering::SeqCst), 5);
        assert_eq!(rx.skip(1), 0);
    }

    // ----------------------------------------------------------- peek/drain

    #[test]
    fn peek_does_not_consume() {
        let (tx, mut rx) = channel::<String, 4>();
        assert!(rx.peek().is_none());
        assert!(tx.push("a".to_string()).is_ok());
        assert_eq!(rx.peek().map(String::as_str), Some("a"));
        assert_eq!(rx.peek().map(String::as_str), Some("a"));
        assert_eq!(rx.pop().as_deref(), Some("a"));
        assert!(rx.peek().is_none());
    }

    #[test]
    fn drain_is_bounded_to_snapshot() {
        let (tx, mut rx) = channel::<u64, 8>();
        for i in 0..5 {
            assert!(tx.push(i).is_ok());
        }
        for (i, v) in rx.drain().enumerate() {
            assert_eq!(v, i as u64);
            assert!(tx.push(100 + v).is_ok()); // does not extend the snapshot
        }
        let rest: Vec<u64> = rx.drain().collect();
        assert_eq!(rest, [100, 101, 102, 103, 104]);
    }

    struct Liar {
        yields: usize,
        claimed: usize,
    }
    impl Iterator for Liar {
        type Item = Box<u64>;
        fn next(&mut self) -> Option<Box<u64>> {
            (self.yields > 0).then(|| {
                self.yields -= 1;
                Box::new(7)
            })
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.claimed, Some(self.claimed))
        }
    }
    impl ExactSizeIterator for Liar {}

    #[test]
    fn lying_iterator_abandons_without_wedging_or_leaking_capacity() {
        let (tx, mut rx) = channel::<Box<u64>, 4>();
        assert_eq!(
            tx.push_iter(&mut Liar {
                yields: 1,
                claimed: 3
            })
            .unwrap(),
            1
        );

        assert_eq!(rx.pop().as_deref(), Some(&7)); // ticket 0
        assert_eq!(rx.pop(), None); // tickets 1-2 skipped, then Empty

        // The full capacity is back: the skip returned the credits.
        for i in 0..4 {
            assert!(tx.push(Box::new(i)).is_ok());
        }
        for i in 0..4 {
            assert_eq!(rx.pop().as_deref(), Some(&i));
        }
    }

    #[test]
    fn drain_snapshot_survives_abandons_beyond_it() {
        let (tx, mut rx) = channel::<Box<u64>, 8>();
        // Tickets 0-1 abandoned BEFORE the snapshot.
        assert_eq!(
            tx.push_iter(&mut Liar {
                yields: 0,
                claimed: 2
            })
            .unwrap(),
            0
        );

        let drain = rx.drain(); // snapshot: remaining = 2
        // Contiguous abandons PAST the snapshot, then a real message behind them.
        assert_eq!(
            tx.push_iter(&mut Liar {
                yields: 0,
                claimed: 2
            })
            .unwrap(),
            0
        );
        tx.push(Box::new(42)).unwrap(); // ticket 4, published, OUTSIDE the snapshot

        // skip_abandoned skips 4 slots against a remaining of 2:
        //   `-=`           -> underflow, panics with overflow checks on
        //   wrapping_sub   -> remaining ~2^64, the drain yields 42 past its snapshot
        //   saturating_sub -> remaining = 0, nothing comes out
        let drained: Vec<_> = drain.collect();
        assert!(
            drained.is_empty(),
            "drain went past its snapshot: {drained:?}"
        );

        assert_eq!(rx.pop().as_deref(), Some(&42));
        assert_eq!(rx.pop(), None);
    }

    struct PanicAfter {
        yields: usize,
    }
    impl Iterator for PanicAfter {
        type Item = Box<u64>;
        fn next(&mut self) -> Option<Box<u64>> {
            if self.yields == 0 {
                panic!("iterator panics mid-run")
            }
            self.yields -= 1;
            Some(Box::new(7))
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (3, Some(3))
        }
    }
    impl ExactSizeIterator for PanicAfter {}

    #[test]
    fn panicking_iterator_abandons_without_wedging_or_leaking_capacity() {
        let (tx, mut rx) = channel::<Box<u64>, 4>();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = tx.push_iter(&mut PanicAfter { yields: 1 });
        }));
        assert!(caught.is_err());

        assert_eq!(rx.pop().as_deref(), Some(&7)); // written before the panic: published
        assert_eq!(rx.pop(), None); // abandoned tickets: skipped

        // The full capacity is back.
        for i in 0..4 {
            assert!(tx.push(Box::new(i)).is_ok());
        }
        for i in 0..4 {
            assert_eq!(rx.pop().as_deref(), Some(&i));
        }
    }

    // ----------------------------------------------------------- disconnect

    #[test]
    fn push_fails_fast_once_consumer_gone() {
        let (tx, rx) = channel::<u64, 4>();
        drop(rx);
        match tx.push(7) {
            Err(Error::ReceiverDisconnected(returned)) => assert_eq!(returned.into_value(), 7),
            other => panic!("expected ReceiverDisconnected, got {other:?}"),
        }
        assert!(tx.push_batch(&[1, 2]).is_err());
    }

    #[test]
    fn consumer_sees_close_after_last_producer_drop() {
        let (tx, rx) = channel::<u64, 4>();
        let tx2 = tx.clone();
        drop(tx);
        assert!(!rx.is_closed());
        drop(tx2);
        assert!(rx.is_closed());
    }
}
