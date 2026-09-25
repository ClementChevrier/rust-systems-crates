#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(not(loom))]
mod std_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::{error::*, spsc::channel};

    const STRESS_MESSAGES: u64 = 1_000_000;
    const MIRI_STRESS_MESSAGES: u64 = 1_000;

    fn stress_messages() -> u64 {
        if cfg!(miri) {
            MIRI_STRESS_MESSAGES
        } else {
            STRESS_MESSAGES
        }
    }

    #[test]
    fn is_full_reports_both_states() {
        let (mut tx, mut rx) = channel::<u64, 2>();
        assert!(!tx.is_full());
        assert!(tx.push(1).is_ok());
        assert!(!tx.is_full());
        assert!(tx.push(2).is_ok());
        assert!(tx.is_full());
        assert_eq!(rx.pop(), Some(1));
        assert!(!tx.is_full());
    }

    #[test]
    fn fifo_roundtrip_with_wraparound() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        let rounds: u64 = if cfg!(miri) { 64 } else { 4096 };
        for round in 0..rounds {
            assert!(tx.push(round).is_ok());
            assert_eq!(rx.pop(), Some(round));
        }
        assert!(rx.is_empty());
    }

    #[test]
    fn full_returns_value_and_empty_returns_none() {
        let (mut tx, mut rx) = channel::<u64, 4>();
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
    fn push_batch_fills_free_space_from_empty() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        assert_eq!(tx.push_batch(&[1, 2, 3, 4, 5, 6]).unwrap(), 4);
        assert_eq!(tx.push_batch(&[7]).unwrap(), 0);
        for i in 1..=4 {
            assert_eq!(rx.pop(), Some(i));
        }
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn push_batch_partial_when_nearly_full() {
        let (mut tx, _rx) = channel::<u64, 4>();
        assert!(tx.push(0).is_ok());
        assert_eq!(tx.push_batch(&[1, 2, 3, 4, 5]).unwrap(), 3);
        assert!(tx.is_full());
    }

    #[test]
    fn push_batch_straddles_wrap_boundary() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        assert_eq!(tx.push_batch(&[0, 0, 0]).unwrap(), 3);
        let mut sink = [0u64; 3];
        assert_eq!(rx.pop_batch(&mut sink), 3);
        // head = tail = 3: the next batch straddles the wrap at index 4.
        assert_eq!(tx.push_batch(&[10, 11, 12, 13]).unwrap(), 4);
        let mut out = [0u64; 4];
        assert_eq!(rx.pop_batch(&mut out), 4);
        assert_eq!(out, [10, 11, 12, 13]);
    }

    #[test]
    fn drops_each_element_exactly_once() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        {
            let (mut tx, mut rx) = channel::<Counted, 8>();
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
        // The 3 residues are dropped by Drop for Ring.
        assert_eq!(drops.load(Ordering::SeqCst), 9);
    }

    #[test]
    fn consumer_drains_after_producer_drop_then_observes_close() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        assert!(tx.push(1).is_ok());
        assert!(tx.push(2).is_ok());
        drop(tx);
        assert!(rx.is_closed());
        assert_eq!(rx.pop(), Some(1));
        assert_eq!(rx.pop(), Some(2));
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn push_fails_fast_once_consumer_gone() {
        let (mut tx, rx) = channel::<u64, 4>();
        drop(rx);
        match tx.push(7) {
            Err(Error::ReceiverDisconnected(returned)) => assert_eq!(returned.into_value(), 7),
            other => panic!("expected ReceiverDisconnected, got {other:?}"),
        }
        assert!(tx.push_batch(&[1, 2]).is_err());
    }

    #[test]
    fn threaded_conservation_u64() {
        let n = stress_messages();
        let (mut tx, mut rx) = channel::<u64, 1024>();

        let producer = std::thread::spawn(move || {
            for i in 0..n {
                while tx.push(i).is_err() {
                    std::thread::yield_now();
                }
            }
        });

        let mut expected = 0u64;
        while expected < n {
            match rx.pop() {
                Some(v) => {
                    assert_eq!(v, expected, "value out of order");
                    expected += 1;
                }
                None => std::thread::yield_now(),
            }
        }
        producer.join().unwrap();
    }

    #[test]
    fn threaded_conservation_heap_payload() {
        let n = (stress_messages() / 4) as usize;
        let (mut tx, mut rx) = channel::<Box<usize>, 8>();

        let producer = std::thread::spawn(move || {
            for i in 0..n {
                let mut item = Box::new(i);
                loop {
                    match tx.push(item) {
                        Ok(()) => break,
                        Err(Error::ChannelFull(back)) => {
                            item = back;
                            std::thread::yield_now();
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            }
        });

        let mut expected = 0usize;
        while expected < n {
            match rx.pop() {
                Some(v) => {
                    assert_eq!(*v, expected);
                    expected += 1;
                }
                None => std::thread::yield_now(),
            }
        }
        producer.join().unwrap();
    }

    #[test]
    fn threaded_batch_conservation() {
        const CHUNK: usize = 32;
        let n = stress_messages();
        let (mut tx, mut rx) = channel::<u64, 128>();

        let producer = std::thread::spawn(move || {
            let mut next = 0u64;
            let mut chunk = [0u64; CHUNK];
            while next < n {
                let want = ((n - next) as usize).min(CHUNK);
                for (i, c) in chunk[..want].iter_mut().enumerate() {
                    *c = next + i as u64;
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
        });

        let mut expected = 0u64;
        let mut buf = [0u64; CHUNK];
        while expected < n {
            let got = rx.pop_batch(&mut buf);
            if got == 0 {
                std::thread::yield_now();
                continue;
            }
            for &v in &buf[..got] {
                assert_eq!(v, expected);
                expected += 1;
            }
        }
        producer.join().unwrap();
    }

    #[test]
    fn drain_works_after_wraparound() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        for i in 0..10 {
            assert!(tx.push(i).is_ok());
            assert_eq!(rx.pop(), Some(i));
        }
        // head = tail = 10, empty ring: drain must yield nothing
        assert_eq!(rx.drain().count(), 0);

        assert!(tx.push(42).is_ok());
        let drained: Vec<u64> = rx.drain().collect();
        assert_eq!(drained, [42]);
    }

    #[test]
    fn drain_is_bounded_to_snapshot() {
        let (mut tx, mut rx) = channel::<u64, 8>();
        for i in 0..5 {
            assert!(tx.push(i).is_ok());
        }
        for (i, v) in rx.drain().enumerate() {
            assert_eq!(v, i as u64);
            // pushing during the drain does not extend the snapshot
            assert!(tx.push(100 + v).is_ok());
        }
        assert_eq!(rx.len(), 5);
        let rest: Vec<u64> = rx.drain().collect();
        assert_eq!(rest, [100, 101, 102, 103, 104]);
    }

    #[test]
    fn drain_partial_keeps_rest_in_queue() {
        let (mut tx, mut rx) = channel::<String, 8>();
        for i in 0..4 {
            assert!(tx.push(i.to_string()).is_ok());
        }
        let taken: Vec<String> = rx.drain().take(2).collect();
        assert_eq!(taken, ["0", "1"]);
        assert_eq!(rx.len(), 2);
        assert_eq!(rx.pop().as_deref(), Some("2"));
    }

    #[test]
    fn drain_drops_each_element_exactly_once() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        {
            let (mut tx, mut rx) = channel::<Counted, 8>();
            for _ in 0..6 {
                assert!(tx.push(Counted(Arc::clone(&drops))).is_ok());
            }
            // 2 consumed by the partial drain, 4 left in the ring
            rx.drain().take(2).for_each(drop);
            assert_eq!(drops.load(Ordering::SeqCst), 2);
        }
        // the 4 residues are dropped by Drop for Ring
        assert_eq!(drops.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn peek_does_not_consume() {
        let (mut tx, mut rx) = channel::<String, 4>();
        assert!(rx.peek().is_none());
        assert!(tx.push("a".to_string()).is_ok());
        assert_eq!(rx.peek().map(String::as_str), Some("a"));
        assert_eq!(rx.peek().map(String::as_str), Some("a"));
        assert_eq!(rx.len(), 1);
        assert_eq!(rx.pop().as_deref(), Some("a"));
        assert!(rx.peek().is_none());
    }

    #[test]
    fn read_chunk_views_and_commits_across_wrap() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        for i in 0..3 {
            assert!(tx.push(i).is_ok());
            assert_eq!(rx.pop(), Some(i));
        }
        for i in 10..13 {
            assert!(tx.push(i).is_ok());
        }

        let chunk = rx.read_chunk();
        assert_eq!(chunk.len(), 3);
        let (first, second) = chunk.as_slices();
        assert_eq!(first, [10]);
        assert_eq!(second, [11, 12]);
        let seen: Vec<u64> = chunk.iter().copied().collect();
        assert_eq!(seen, [10, 11, 12]);
        chunk.commit(2);

        assert_eq!(rx.len(), 1);
        assert_eq!(rx.pop(), Some(12));
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn read_chunk_commit_clamped_and_drop_consumes_nothing() {
        let (mut tx, mut rx) = channel::<u64, 4>();
        for i in 0..3 {
            assert!(tx.push(i).is_ok());
        }
        assert_eq!(rx.len(), 3);

        rx.read_chunk().commit(99); // clamped to the view
        assert!(rx.is_empty());

        rx.read_chunk().commit_all(); // on an empty view: no-op
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn push_iter_moves_non_copy_until_full() {
        let (mut tx, mut rx) = channel::<String, 4>();
        assert!(tx.push("x".to_string()).is_ok());

        let mut source = (0..10).map(|i| i.to_string());
        assert_eq!(tx.push_iter(&mut source).unwrap(), 3); // 4 - 1 already present
        assert_eq!(source.next().as_deref(), Some("3")); // the iterator resumes right after

        assert_eq!(rx.pop().as_deref(), Some("x"));
        assert_eq!(rx.pop().as_deref(), Some("0"));
        assert_eq!(rx.pop().as_deref(), Some("1"));
        assert_eq!(rx.pop().as_deref(), Some("2"));
        assert_eq!(rx.pop(), None);

        drop(rx);
        assert!(tx.push_iter(&mut (0..3).map(|i| i.to_string())).is_err());
    }

    #[test]
    fn skip_and_clear_drop_each_skipped_element() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let (mut tx, mut rx) = channel::<Counted, 8>();
        for _ in 0..5 {
            assert!(tx.push(Counted(Arc::clone(&drops))).is_ok());
        }

        assert_eq!(rx.skip(2), 2);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(rx.clear(), 3);
        assert_eq!(drops.load(Ordering::SeqCst), 5);
        assert_eq!(rx.skip(1), 0);
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
    fn panicking_iterator_publishes_what_it_wrote_and_keeps_capacity() {
        let (mut tx, mut rx) = channel::<Box<u64>, 4>();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = tx.push_iter(&mut PanicAfter { yields: 1 });
        }));
        assert!(caught.is_err());

        assert_eq!(rx.pop().as_deref(), Some(&7)); // written before the panic: published
        assert_eq!(rx.pop(), None); // nothing else was written

        // The full capacity is available.
        for i in 0..4 {
            assert!(tx.push(Box::new(i)).is_ok());
        }
        for i in 0..4 {
            assert_eq!(rx.pop().as_deref(), Some(&i));
        }
    }
}
