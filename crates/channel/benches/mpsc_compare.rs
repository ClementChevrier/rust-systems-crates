//! Throughput of `channel::mpsc` against the Vyukov baseline,
//! `std::sync::mpsc`, crossbeam `ArrayQueue` and crossbeam-channel.
//!
//! Nightly only (`#[bench]`):
//! `cargo +nightly bench -p channel --bench mpsc_compare --features "nightly-benches crossbeam-queue crossbeam-channel"`

#![feature(test)]

extern crate test;

use std::sync::{Arc, Barrier};

use test::{Bencher, black_box};

use channel::{Error, mpsc::channel};
use wincore::thread::{Builder, affinity::ProcessorId, control::pin_current_thread_to_cpu};

const RING_CAPACITY: usize = 1024;
const MESSAGES: u64 = 1_000_000; // total, split across producers
const BATCH: usize = 64;

const CONSUMER_CPU: ProcessorId = ProcessorId::try_new(4, 0).unwrap();
// 4 physical cores (0,2,4,6); odd CPUs are HT siblings. Core 4 is reserved
// for the consumer (its sibling 5 must stay empty), core 0 hosts most OS
// interrupt activity so it is used last.
const PRODUCER_CPUS: [ProcessorId; 3] = [
    ProcessorId::try_new(2, 0).unwrap(),
    ProcessorId::try_new(6, 0).unwrap(),
    ProcessorId::try_new(0, 0).unwrap(),
];

fn pin_consumer() {
    if pin_current_thread_to_cpu(CONSUMER_CPU).is_err() {
        eprintln!("bench: consumer pinning refused (CPU {CONSUMER_CPU})");
    }
}

fn spawn_pinned(idx: usize, f: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<()> {
    let cpu = PRODUCER_CPUS[idx % PRODUCER_CPUS.len()];
    let Ok(rslt_spawn) = Builder::new()
        .name(format!("bench_prod_{idx}"))
        .pin_to(cpu)
        .spawn(f)
    else {
        panic!("bench: cannot spawn or pin producer {idx} (CPU {cpu})")
    };
    rslt_spawn.handle
}
fn spawn_pinned_oversub(
    idx: usize,
    f: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    let cpu = OVERSUB_CPUS[idx % OVERSUB_CPUS.len()];
    let Ok(rslt_spawn) = Builder::new()
        .name(format!("bench_prod_{idx}"))
        .pin_to(cpu)
        .spawn(f)
    else {
        panic!("bench: cannot spawn or pin producer {idx} (CPU {cpu})")
    };
    rslt_spawn.handle
}

// ------------------------------------------------------- channel::mpsc

fn run_my_mpsc(producers: usize, total: u64) {
    let (tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(producers + 1));
    let per = total / producers as u64;

    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let tx = tx.clone();
            let b = Arc::clone(&barrier);
            spawn_pinned(p, move || {
                b.wait();
                for i in 0..per {
                    let mut v = i;
                    loop {
                        match tx.push(v) {
                            Ok(()) => break,
                            Err(Error::ChannelFull(back)) => {
                                v = back;
                                std::hint::spin_loop();
                            }
                            Err(e) => panic!("{e:?}"),
                        }
                    }
                }
            })
        })
        .collect();
    drop(tx);

    barrier.wait();
    let goal = per * producers as u64;
    let mut received = 0u64;
    while received < goal {
        match rx.pop() {
            Some(v) => {
                black_box(v);
                received += 1;
            }
            None => std::hint::spin_loop(),
        }
    }
    for h in handles {
        h.join().unwrap();
    }
}

fn run_my_mpsc_oversub(producers: usize, total: u64) {
    let (tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(producers + 1));
    let per = total / producers as u64;

    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let tx = tx.clone();
            let b = Arc::clone(&barrier);
            spawn_pinned_oversub(p, move || {
                b.wait();
                for i in 0..per {
                    let mut v = i;
                    loop {
                        match tx.push(v) {
                            Ok(()) => break,
                            Err(e) => {
                                v = e.into_value();
                                std::thread::yield_now();
                            }
                        }
                    }
                }
            })
        })
        .collect();
    drop(tx);

    barrier.wait();
    let goal = per * producers as u64;
    let mut received = 0u64;
    while received < goal {
        match rx.pop() {
            Some(v) => {
                black_box(v);
                received += 1;
            }
            None => std::hint::spin_loop(),
        }
    }
    for h in handles {
        h.join().unwrap();
    }
}

fn run_my_mpsc_batch(producers: usize, total: u64) {
    let (tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(producers + 1));
    let per = total / producers as u64;

    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let tx = tx.clone();
            let b = Arc::clone(&barrier);
            spawn_pinned(p, move || {
                b.wait();
                let mut chunk = [0u64; BATCH];
                let mut next = 0u64;
                while next < per {
                    let want = ((per - next) as usize).min(BATCH);
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
        })
        .collect();
    drop(tx);

    barrier.wait();
    let goal = per * producers as u64;
    let mut received = 0u64;
    while received < goal {
        match rx.pop() {
            Some(v) => {
                black_box(v);
                received += 1;
            }
            None => std::hint::spin_loop(),
        }
    }
    for h in handles {
        h.join().unwrap();
    }
}

// ns/iter = instruction floor of push + pop, no contention, no coherence.
#[bench]
fn mpsc_same_thread_roundtrip(b: &mut Bencher) {
    pin_consumer();
    let (tx, mut rx) = channel::<u64, RING_CAPACITY>();
    let mut i = 0u64;
    b.iter(|| {
        i = i.wrapping_add(1);
        black_box(tx.push(i).is_ok());
        black_box(rx.pop())
    });
}

// One producer = the cost of generalising to MPSC, against the SPSC (compare
// with the spsc_compare figures on the same machine).
#[bench]
fn mpsc_cross_thread_1p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_my_mpsc(1, MESSAGES));
}

#[bench]
fn mpsc_cross_thread_2p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_my_mpsc(2, MESSAGES));
}

#[bench]
fn mpsc_cross_thread_3p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_my_mpsc(3, MESSAGES));
}

#[bench]
fn mpsc_cross_thread_batch64_2p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_my_mpsc_batch(2, MESSAGES));
}

// Production-shaped run: more producer threads than available logical CPUs
// (feed handlers + services in the real deployment). Two threads pinned to
// the same logical CPU force OS timeslicing => real preemption between
// ticket reservation and publication (the Busy / head-of-line case).
// The consumer's core pair (4/5) is deliberately excluded.
const OVERSUB_CPUS: [ProcessorId; 6] = [
    ProcessorId::try_new(2, 0).unwrap(),
    ProcessorId::try_new(3, 0).unwrap(),
    ProcessorId::try_new(6, 0).unwrap(),
    ProcessorId::try_new(7, 0).unwrap(),
    ProcessorId::try_new(0, 0).unwrap(),
    ProcessorId::try_new(1, 0).unwrap(),
];
const OVERSUB_PRODUCERS: usize = 8; // 8 threads on 6 logical CPUs

#[bench]
fn mpsc_cross_thread_8p_oversubscribed(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_my_mpsc_oversub(OVERSUB_PRODUCERS, MESSAGES));
}

// ------------------------------------------------- baseline: Vyukov

fn run_vyukov(producers: usize, total: u64) {
    use channel::mpsc::ref_vyukov;

    let (tx, mut rx) = ref_vyukov::channel::<u64, RING_CAPACITY>();
    let barrier = Arc::new(Barrier::new(producers + 1));
    let per = total / producers as u64;

    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let tx = tx.clone();
            let b = Arc::clone(&barrier);
            spawn_pinned(p, move || {
                b.wait();
                for i in 0..per {
                    let mut v = i;
                    loop {
                        match tx.push(v) {
                            Ok(()) => break,
                            Err(e) => {
                                v = e.into_value();
                                std::hint::spin_loop();
                            }
                        }
                    }
                }
            })
        })
        .collect();
    drop(tx);

    barrier.wait();
    let goal = per * producers as u64;
    let mut received = 0u64;
    while received < goal {
        match rx.pop() {
            Some(v) => {
                black_box(v);
                received += 1;
            }
            None => std::hint::spin_loop(),
        }
    }
    for h in handles {
        h.join().unwrap();
    }
}

#[bench]
fn vyukov_cross_thread_1p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_vyukov(1, MESSAGES));
}

#[bench]
fn vyukov_cross_thread_2p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_vyukov(2, MESSAGES));
}

#[bench]
fn vyukov_cross_thread_3p(b: &mut Bencher) {
    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| run_vyukov(3, MESSAGES));
}

// ---------------------------------------------------- external references

#[bench]
fn std_mpsc_try_cross_thread_3p(b: &mut Bencher) {
    use std::sync::mpsc::{TryRecvError, TrySendError, sync_channel};

    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| {
        let (tx, rx) = sync_channel::<u64>(RING_CAPACITY);
        let barrier = Arc::new(Barrier::new(4));
        let per = MESSAGES / 3;

        let handles: Vec<_> = (0..3)
            .map(|p| {
                let tx = tx.clone();
                let b = Arc::clone(&barrier);
                spawn_pinned(p, move || {
                    b.wait();
                    for i in 0..per {
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
                })
            })
            .collect();
        drop(tx);

        barrier.wait();
        let mut received = 0u64;
        while received < per * 3 {
            match rx.try_recv() {
                Ok(v) => {
                    black_box(v);
                    received += 1;
                }
                Err(TryRecvError::Empty) => std::hint::spin_loop(),
                Err(e) => panic!("{e}"),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
    });
}

#[cfg(feature = "crossbeam-queue")]
#[bench]
fn crossbeam_array_queue_3p(b: &mut Bencher) {
    use crossbeam_queue::ArrayQueue;

    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| {
        // Same algorithm as ref_vyukov, written by the crossbeam team. Note that
        // its pop pays for the multi-consumer CAS, which the MPSC versions do not.
        let q = Arc::new(ArrayQueue::<u64>::new(RING_CAPACITY));
        let barrier = Arc::new(Barrier::new(4));
        let per = MESSAGES / 3;

        let handles: Vec<_> = (0..3)
            .map(|p| {
                let q = Arc::clone(&q);
                let b = Arc::clone(&barrier);
                spawn_pinned(p, move || {
                    b.wait();
                    for i in 0..per {
                        let mut v = i;
                        while let Err(back) = q.push(v) {
                            v = back;
                            std::hint::spin_loop();
                        }
                    }
                })
            })
            .collect();

        barrier.wait();
        let mut received = 0u64;
        while received < per * 3 {
            match q.pop() {
                Some(v) => {
                    black_box(v);
                    received += 1;
                }
                None => std::hint::spin_loop(),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
    });
}

#[cfg(feature = "crossbeam-channel")]
#[bench]
fn crossbeam_channel_bounded_3p(b: &mut Bencher) {
    use crossbeam_channel::{TryRecvError, TrySendError, bounded};

    pin_consumer();
    b.bytes = MESSAGES * size_of::<u64>() as u64;
    b.iter(|| {
        let (tx, rx) = bounded::<u64>(RING_CAPACITY);
        let barrier = Arc::new(Barrier::new(4));
        let per = MESSAGES / 3;

        let handles: Vec<_> = (0..3)
            .map(|p| {
                let tx = tx.clone();
                let b = Arc::clone(&barrier);
                spawn_pinned(p, move || {
                    b.wait();
                    for i in 0..per {
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
                })
            })
            .collect();
        drop(tx);

        barrier.wait();
        let mut received = 0u64;
        while received < per * 3 {
            match rx.try_recv() {
                Ok(v) => {
                    black_box(v);
                    received += 1;
                }
                Err(TryRecvError::Empty) => std::hint::spin_loop(),
                Err(e) => panic!("{e}"),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
    });
}
