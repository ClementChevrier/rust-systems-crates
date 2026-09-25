//! Breaks down the caller-side cost of `log()`, to decide on the "inline,
//! zero-allocation body" option (option 3):
//!
//!   part_*  the pieces in isolation. NO file is written, clean results.
//!           option 3 gain ~ (part_body_heap - part_body_inline)
//!                             - (part_push_288b - part_push_64b)
//!   log_*   the real path end to end (global logger + file).
//!           WARNING: writes real logs under %TEMP% (the path is printed at
//!           startup; delete it afterwards). If the worker falls behind,
//!           drops skew the measurement: check that the produced file holds
//!           no "dropped N messages" line.
//!
//! Run with:
//!   cargo +nightly bench -p logger --features nightly-benches -- part_
//!   cargo +nightly bench -p logger --features nightly-benches -- log_
#![feature(test)]
extern crate test;

use std::{
    fmt::Write as _,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

use test::{Bencher, black_box};

use channel::{Error, mpsc};
use logger::*;
use wincore::thread::{Builder, affinity::ProcessorId, control::pin_current_thread_to_cpu};

// Same convention as the channel benches: distinct physical cores, odd CPUs
// are the SMT siblings.
const BENCH_CPU: ProcessorId = ProcessorId::try_new(2, 0).unwrap();
const WORKER_CPU: ProcessorId = ProcessorId::try_new(4, 0).unwrap();
const PRESSURE_CPUS: [ProcessorId; 2] = [
    ProcessorId::try_new(6, 0).unwrap(),
    ProcessorId::try_new(0, 0).unwrap(),
];

const RING_CAPACITY: usize = 1024;
/// Option 3 candidate: the body stored inline in the ring slot.
const INLINE_CAP: usize = 240;

// A message representative of real use in the engine.
const ARG_WORKER: u32 = 3;
const ARG_TICKER: &str = "BTC-USD@1m";
const ARG_PRICE: f64 = 42_123.51;
const STATIC_MSG: &str = "Router is closed now shutting down";

fn pin_bench_thread() {
    if pin_current_thread_to_cpu(BENCH_CPU).is_err() {
        eprintln!("bench: pinning refused (CPU {BENCH_CPU})");
    }
}

// ------------------------------------------------------------- part_* ----

#[bench]
fn part_ts_systemtime_now(b: &mut Bencher) {
    pin_bench_thread();
    b.iter(|| black_box(SystemTime::now()));
}

/// The CURRENT path: String::with_capacity(256) + format + into_bytes.
#[bench]
fn part_body_heap(b: &mut Bencher) {
    pin_bench_thread();
    b.iter(|| {
        let mut s = String::with_capacity(256);
        let _ = write!(
            s,
            "Worker-{} processing candle: {} o={}",
            black_box(ARG_WORKER),
            black_box(ARG_TICKER),
            black_box(ARG_PRICE)
        );
        black_box(s.into_bytes())
    });
}

/// Theoretical floor: same format, reused buffer (no allocation).
/// part_body_heap - part_body_heap_reused = the allocator's share.
#[bench]
fn part_body_heap_reused(b: &mut Bencher) {
    pin_bench_thread();
    let mut s = String::with_capacity(256);
    b.iter(|| {
        s.clear();
        let _ = write!(
            s,
            "Worker-{} processing candle: {} o={}",
            black_box(ARG_WORKER),
            black_box(ARG_TICKER),
            black_box(ARG_PRICE)
        );
        black_box(s.len())
    });
}

/// Option 3 simulated: format straight into an existing [u8; INLINE_CAP]
/// (like the ring slot). Truncates on overflow.
struct InlineWriter {
    buf: [u8; INLINE_CAP],
    len: usize,
}
impl std::fmt::Write for InlineWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let bytes = s.as_bytes();
        let n = bytes.len().min(INLINE_CAP - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
        if n < bytes.len() {
            Err(std::fmt::Error)
        } else {
            Ok(())
        }
    }
}
impl InlineWriter {}

#[bench]
fn part_body_inline(b: &mut Bencher) {
    pin_bench_thread();
    let mut w = InlineWriter {
        buf: [0; INLINE_CAP],
        len: 0,
    };
    b.iter(|| {
        w.len = 0;
        let _ = write!(
            w,
            "Worker-{} processing candle: {} o={}",
            black_box(ARG_WORKER),
            black_box(ARG_TICKER),
            black_box(ARG_PRICE)
        );
        black_box(w.len)
    });
}

/// Option 3's static fast path: a memcpy of the &'static str, no formatting.
#[bench]
fn part_body_static_copy(b: &mut Bencher) {
    pin_bench_thread();
    let mut w = InlineWriter {
        buf: [0; INLINE_CAP],
        len: 0,
    };
    b.iter(|| {
        w.len = 0;
        let _ = w.write_str(black_box(STATIC_MSG));
        black_box(w.len)
    });
}

/// MPSC push with a consumer draining on another physical core.
fn bench_push<T: Copy + Send + 'static>(b: &mut Bencher, value: T) {
    pin_bench_thread();
    let (tx, rx) = mpsc::channel::<T, RING_CAPACITY>();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);

    let consumer = Builder::new()
        .name("bench_drain".to_string())
        .pin_to(WORKER_CPU)
        .spawn(move || {
            let mut rx = rx;
            while !flag.load(Ordering::Relaxed) {
                while rx.pop().is_some() {}
                std::hint::spin_loop();
            }
        })
        .expect("spawn the bench consumer");

    b.iter(|| {
        let mut v = value;
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
    });

    stop.store(true, Ordering::Relaxed);
    drop(tx);
    consumer.handle.join().expect("join consumer");
}

/// ~64 B: the size of the CURRENT LogCommand (body = Vec).
#[bench]
fn part_push_64b(b: &mut Bencher) {
    bench_push(b, [0u64; 8]);
}

/// ~288 B: the size of the LogCommand AFTER option 3 (240-byte inline body + metadata).
#[bench]
fn part_push_288b(b: &mut Bencher) {
    bench_push(b, [0u64; 36]);
}

// -------------------------------------------------------------- log_* ----

fn init_logger() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("logger_bench_{}", std::process::id()));
        eprintln!(
            "bench: logs written to {} (delete afterwards)",
            dir.display()
        );
        let logger = logger::Logger::builder()
            .output(&dir, "bench")
            .min_level(LogLevel::Info)
            .thread_init(|| {
                let _ = pin_current_thread_to_cpu(WORKER_CPU);
            })
            .build()
            .expect("start the bench logger");
        // No shutdown: the worker drains until the bench process ends.
        std::mem::forget(logger);
    });
}

/// The complete real path: allowed + now + format + alloc + push.
#[bench]
fn log_typical_info(b: &mut Bencher) {
    init_logger();
    pin_bench_thread();
    b.iter(|| {
        log_info!(
            "BENCH",
            "Worker-{} processing candle: {} o={}",
            black_box(ARG_WORKER),
            black_box(ARG_TICKER),
            black_box(ARG_PRICE)
        )
    });
}

/// Constant message: a candidate for option 3's as_str() fast path.
#[bench]
fn log_static_info(b: &mut Bencher) {
    init_logger();
    pin_bench_thread();
    b.iter(|| log_info!("BENCH", "Router is closed now shutting down"));
}

/// Filtered level (Debug < Info): the cost of the guard alone, ~1 ns expected.
#[bench]
fn log_filtered_debug(b: &mut Bencher) {
    init_logger();
    pin_bench_thread();
    b.iter(|| log_debug!("BENCH", "never formatted nor sent {}", black_box(ARG_PRICE)));
}

/// Option 3's jitter argument: the same log_typical while two threads
/// saturate the allocator. Compare mean AND spread (+/-) with
/// log_typical_info: that gap is what the inline body removes.
#[bench]
fn log_typical_under_alloc_pressure(b: &mut Bencher) {
    init_logger();
    pin_bench_thread();
    let stop = Arc::new(AtomicBool::new(false));
    let churners: Vec<_> = PRESSURE_CPUS
        .iter()
        .map(|&cpu| {
            let flag = Arc::clone(&stop);
            Builder::new()
                .pin_to(cpu)
                .spawn(move || {
                    let mut keep: Vec<Vec<u8>> = Vec::new();
                    while !flag.load(Ordering::Relaxed) {
                        keep.push(vec![0u8; 64]);
                        if keep.len() > 1024 {
                            keep.clear();
                        }
                    }
                })
                .expect("spawn the allocator churner")
        })
        .collect();

    b.iter(|| {
        log_info!(
            "BENCH",
            "Worker-{} processing candle: {} o={}",
            black_box(ARG_WORKER),
            black_box(ARG_TICKER),
            black_box(ARG_PRICE)
        )
    });

    stop.store(true, Ordering::Relaxed);
    for c in churners {
        c.handle.join().expect("join churner");
    }
}
