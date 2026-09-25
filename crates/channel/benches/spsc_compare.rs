//! Throughput of `channel::spsc` against `std::sync::mpsc` and `rtrb`.
//!
//! Nightly only (`#[bench]`):
//! `cargo +nightly bench -p channel --bench spsc_compare --features "nightly-benches rtrb"`

#![feature(test)]

extern crate test;

use std::sync::{Arc, Barrier};

use test::{Bencher, black_box};

use channel::spsc::{Producer, channel};
use wincore::thread::{Builder, affinity::ProcessorId, control::pin_current_thread_to_cpu};

const RING_CAPACITY: usize = 1024;
const MESSAGES: u64 = 1_000_000;
const EVENT_MESSAGES: u64 = 500_000;
const BLOCKING_MESSAGES: u64 = 200_000;
const BATCH: usize = 64;

const PRODUCER_CPU: ProcessorId = ProcessorId::try_new(2, 0).unwrap();
const CONSUMER_CPU: ProcessorId = ProcessorId::try_new(4, 0).unwrap();

fn pin_consumer() {
    if pin_current_thread_to_cpu(CONSUMER_CPU).is_err() {
        eprintln!("bench: consumer pinning refused (CPU {CONSUMER_CPU})");
    }
}

fn spawn_pinned(f: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<()> {
    let Ok(rslt_spawn) = Builder::new()
        .name("bench_producer".to_string())
        .pin_to(PRODUCER_CPU)
        .spawn(f)
    else {
        panic!("bench: cannot spawn or pin the producer (CPU {PRODUCER_CPU})")
    };
    rslt_spawn.handle
}

fn transfer_single(messages: u64) {
    let (mut tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(2));
    let b = Arc::clone(&barrier);

    let producer = spawn_pinned(move || {
        b.wait();
        for i in 0..messages {
            while tx.push(i).is_err() {
                std::hint::spin_loop();
            }
        }
    });

    barrier.wait();
    let mut received = 0u64;
    while received < messages {
        match rx.pop() {
            Some(v) => {
                black_box(v);
                received += 1;
            }
            None => std::hint::spin_loop(),
        }
    }
    producer.join().unwrap();
}

fn transfer_batch(messages: u64) {
    let (mut tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(2));
    let b = Arc::clone(&barrier);

    let producer = spawn_pinned(move || {
        b.wait();
        let mut chunk = [0u64; BATCH];
        let mut next = 0u64;
        while next < messages {
            let want = ((messages - next) as usize).min(BATCH);
            for (i, c) in chunk[..want].iter_mut().enumerate() {
                *c = next + i as u64;
            }
            let mut sent = 0;
            while sent < want {
                match tx.push_batch(&chunk[sent..want]) {
                    Ok(0) => std::hint::spin_loop(),
                    Ok(pushed) => sent += pushed,
                    Err(e) => panic!("{e:?}"),
                }
            }
            next += want as u64;
        }
    });

    barrier.wait();
    let mut buf = [0u64; BATCH];
    let mut received = 0u64;
    while received < messages {
        let n = rx.pop_batch(&mut buf);
        if n == 0 {
            std::hint::spin_loop();
            continue;
        }
        black_box(&buf[..n]);
        received += n as u64;
    }
    producer.join().unwrap();
}

#[derive(Clone, Copy)]
struct Event([u64; 8]);

fn transfer_event(messages: u64) {
    let (mut tx, mut rx) = channel::<Event, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(2));
    let b = Arc::clone(&barrier);

    let producer = spawn_pinned(move || {
        b.wait();
        for i in 0..messages {
            let ev = Event([i; 8]);
            while tx.push(ev).is_err() {
                std::hint::spin_loop();
            }
        }
    });

    barrier.wait();
    let mut received = 0u64;
    while received < messages {
        match rx.pop() {
            Some(v) => {
                black_box(v.0[0]);
                received += 1;
            }
            None => std::hint::spin_loop(),
        }
    }
    producer.join().unwrap();
}

// ns/iter = direct cost of one push + pop with no contention at all:
// the instruction floor of the implementation.
#[bench]
fn spsc_same_thread_roundtrip(b: &mut Bencher) {
    pin_consumer();
    let (mut tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let mut i = 0u64;
    b.iter(|| {
        i = i.wrapping_add(1);
        black_box(tx.push(i).is_ok());
        black_box(rx.pop())
    });
}

// Cross-thread benches: ns/iter / MESSAGES = ns/msg.
// Setting b.bytes makes libtest print the throughput in MB/s.
#[bench]
fn spsc_cross_thread_u64(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| transfer_single(MESSAGES));
}

#[bench]
fn spsc_cross_thread_batch64_u64(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| transfer_batch(MESSAGES));
}

#[bench]
fn spsc_cross_thread_event_64b(b: &mut Bencher) {
    pin_consumer();
    b.bytes = EVENT_MESSAGES * std::mem::size_of::<Event>() as u64;
    b.iter(|| transfer_event(EVENT_MESSAGES));
}

#[bench]
fn std_mpsc_try_cross_thread(b: &mut Bencher) {
    use std::sync::mpsc::{TryRecvError, TrySendError, sync_channel};

    pin_consumer();
    b.bytes = MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| {
        let (tx, rx) = sync_channel::<u64>(RING_CAPACITY);
        let barrier = Arc::new(Barrier::new(2));
        let bar = Arc::clone(&barrier);

        let producer = spawn_pinned(move || {
            bar.wait();
            for i in 0..MESSAGES {
                let mut v = i;
                loop {
                    match tx.try_send(v) {
                        Ok(()) => break,
                        Err(TrySendError::Full(back)) => {
                            v = back;
                            std::hint::spin_loop();
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            }
        });

        barrier.wait();
        let mut received = 0u64;
        while received < MESSAGES {
            match rx.try_recv() {
                Ok(v) => {
                    black_box(v);
                    received += 1;
                }
                Err(TryRecvError::Empty) => std::hint::spin_loop(),
                Err(e) => panic!("{e}"),
            }
        }
        producer.join().unwrap();
    });
}

#[bench]
fn std_mpsc_blocking_cross_thread(b: &mut Bencher) {
    pin_consumer();
    b.bytes = BLOCKING_MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<u64>(RING_CAPACITY);
        let barrier = Arc::new(Barrier::new(2));
        let bar = Arc::clone(&barrier);

        let producer = spawn_pinned(move || {
            bar.wait();
            for i in 0..BLOCKING_MESSAGES {
                tx.send(i).expect("bench: consumer alive");
            }
        });

        barrier.wait();
        for _ in 0..BLOCKING_MESSAGES {
            black_box(rx.recv().expect("bench: producer alive"));
        }
        producer.join().unwrap();
    });
}

fn spawn_batch_producer(
    mut tx: Producer<u64, RING_CAPACITY>,
    messages: u64,
    barrier: Arc<Barrier>,
) -> std::thread::JoinHandle<()> {
    spawn_pinned(move || {
        barrier.wait();
        let mut chunk = [0u64; BATCH];
        let mut next = 0u64;
        while next < messages {
            let want = ((messages - next) as usize).min(BATCH);
            for (i, c) in chunk[..want].iter_mut().enumerate() {
                *c = next + i as u64;
            }
            let mut sent = 0;
            while sent < want {
                match tx.push_batch(&chunk[sent..want]) {
                    Ok(0) => std::hint::spin_loop(),
                    Ok(pushed) => sent += pushed,
                    Err(e) => panic!("{e:?}"),
                }
            }
            next += want as u64;
        }
    })
}

#[bench]
fn consumer_pop_with_batch_producer(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| {
        let (tx, mut rx) = channel::<u64, RING_CAPACITY>();
        let barrier = Arc::new(Barrier::new(2));
        let producer = spawn_batch_producer(tx, MESSAGES, Arc::clone(&barrier));
        barrier.wait();
        let mut received = 0u64;
        while received < MESSAGES {
            match rx.pop() {
                Some(v) => {
                    black_box(v);
                    received += 1;
                }
                None => std::hint::spin_loop(),
            }
        }
        producer.join().unwrap();
    });
}

#[bench]
fn consumer_drain_with_batch_producer(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| {
        let (tx, mut rx) = channel::<u64, RING_CAPACITY>();
        let barrier = Arc::new(Barrier::new(2));
        let producer = spawn_batch_producer(tx, MESSAGES, Arc::clone(&barrier));
        barrier.wait();
        let mut received = 0u64;
        while received < MESSAGES {
            let mut got = 0u64;
            for v in rx.drain() {
                black_box(v);
                got += 1;
            }
            if got == 0 {
                std::hint::spin_loop();
            }
            received += got;
        }
        producer.join().unwrap();
    });
}

const DRAIN_CAP: usize = 1 << 16; // 512 KiB of u64: spills out of L2. Try 1 << 10 as well (fits in L1).

fn refill(tx: &mut Producer<u64, DRAIN_CAP>) {
    let chunk = [1u64; BATCH];
    let mut filled = 0;
    while filled < DRAIN_CAP {
        match tx.push_batch(&chunk) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => panic!("{e:?}"),
        }
    }
}

#[bench]
fn empty_full_ring_pop(b: &mut Bencher) {
    pin_consumer();
    let (mut tx, mut rx) = channel::<u64, DRAIN_CAP>();
    b.bytes = (DRAIN_CAP * std::mem::size_of::<u64>()) as u64;
    b.iter(|| {
        refill(&mut tx);
        while let Some(v) = rx.pop() {
            black_box(v);
        }
    });
}

#[bench]
fn empty_full_ring_drain(b: &mut Bencher) {
    pin_consumer();
    let (mut tx, mut rx) = channel::<u64, DRAIN_CAP>();
    b.bytes = (DRAIN_CAP * std::mem::size_of::<u64>()) as u64;
    b.iter(|| {
        refill(&mut tx);
        for v in rx.drain() {
            black_box(v);
        }
    });
}

#[bench]
fn empty_full_ring_pop_batch_full(b: &mut Bencher) {
    pin_consumer();
    let (mut tx, mut rx) = channel::<u64, DRAIN_CAP>();
    let mut out = vec![0u64; DRAIN_CAP];
    b.bytes = (DRAIN_CAP * std::mem::size_of::<u64>()) as u64;
    b.iter(|| {
        refill(&mut tx);
        loop {
            let n = rx.pop_batch(&mut out);
            if n == 0 {
                break;
            }
            black_box(&out[..n]);
        }
    });
}

#[bench]
fn empty_full_ring_pop_batch_eighth(b: &mut Bencher) {
    pin_consumer();
    let (mut tx, mut rx) = channel::<u64, DRAIN_CAP>();
    let mut out = vec![0u64; DRAIN_CAP / 8];
    b.bytes = (DRAIN_CAP * std::mem::size_of::<u64>()) as u64;
    b.iter(|| {
        refill(&mut tx);
        loop {
            let n = rx.pop_batch(&mut out);
            if n == 0 {
                break;
            }
            black_box(&out[..n]);
        }
    });
}

#[cfg(feature = "rtrb")]
#[bench]
fn rtrb_cross_thread_u64(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * std::mem::size_of::<u64>() as u64;
    b.iter(|| {
        let (mut tx, mut rx) = rtrb::RingBuffer::<u64>::new(RING_CAPACITY);
        let barrier = Arc::new(Barrier::new(2));
        let bar = Arc::clone(&barrier);

        let producer = spawn_pinned(move || {
            bar.wait();
            for i in 0..MESSAGES {
                while tx.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });

        barrier.wait();
        let mut received = 0u64;
        while received < MESSAGES {
            match rx.pop() {
                Ok(v) => {
                    black_box(v);
                    received += 1;
                }
                Err(_) => std::hint::spin_loop(),
            }
        }
        producer.join().unwrap();
    });
}
