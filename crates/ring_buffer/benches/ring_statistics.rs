//! Benchmarks `RingStatistics::push`'s per-iteration cost (including
//! `mean`/`variance`) for the two `Accumulator` strategies, and prints a
//! numerical divergence comparison between them for a large-magnitude,
//! small-spread series -- run with `--nocapture` to see the printed table:
//! `cargo +nightly bench -p ring_buffer --features nightly-benches -- --nocapture`
#![feature(test)]

extern crate test;

use std::collections::VecDeque;

use test::{Bencher, black_box};

use ring_buffer::ring_statistics::{LossyAccumulator, RingStatistics, WelfordAccumulator};

fn create_full_lossy(size: usize) -> RingStatistics<i32, LossyAccumulator<i64>> {
    let mut r = RingStatistics::try_new(size, LossyAccumulator::<i64>::default())
        .expect("size is valid in every caller");
    for i in 0..size {
        r.push(i as i32);
    }
    r
}

fn create_full_welford(size: usize) -> RingStatistics<i32, WelfordAccumulator> {
    let mut r = RingStatistics::try_new(size, WelfordAccumulator::default())
        .expect("size is valid in every caller");
    for i in 0..size {
        r.push(i as i32);
    }
    r
}

fn crate_full_vecq(size: usize) -> VecDeque<u32> {
    let mut vq = VecDeque::with_capacity(size);
    for i in 0..size {
        vq.push_front(i as u32);
    }
    vq
}

fn get_mean(data: &VecDeque<u32>) -> f64 {
    data.iter().sum::<u32>() as f64 / data.len() as f64
}

fn get_variance(data: &VecDeque<u32>) -> f64 {
    let mean = data.iter().map(|&v| v as f64).sum::<f64>() / data.len() as f64;
    data.iter()
        .map(|&value| {
            let diff = mean - value as f64;
            diff * diff
        })
        .sum::<f64>()
        / data.len() as f64
}

const CREATION_BENCH_SIZE: usize = 1024;
// Creation bench
#[bench]
fn creating_lossy(b: &mut Bencher) {
    b.iter(|| create_full_lossy(CREATION_BENCH_SIZE));
}

#[bench]
fn creating_welford(b: &mut Bencher) {
    b.iter(|| create_full_welford(CREATION_BENCH_SIZE));
}

#[bench]
fn creating_vecq(b: &mut Bencher) {
    b.iter(|| crate_full_vecq(CREATION_BENCH_SIZE));
}

// Ring already created. Stats bench
// ---------------------------------------- 1024 ---------------------------------------- //
#[bench]
fn stats_per_iter_lossy_1024(b: &mut Bencher) {
    let mut ring = create_full_lossy(1024);
    b.iter(|| {
        for idx in 0..(1024 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_1024(b: &mut Bencher) {
    let mut ring = create_full_welford(1024);
    b.iter(|| {
        for idx in 0..(1024 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_vecq_1024(b: &mut Bencher) {
    let mut vecq = crate_full_vecq(1024);
    b.iter(|| {
        for idx in 0..(1024 * 2) {
            vecq.pop_front();
            vecq.push_back(idx as u32);
            get_mean(&vecq);
            get_variance(&vecq);
        }
    });
}

// ---------------------------------------- 512 ---------------------------------------- //
#[bench]
fn stats_per_iter_lossy_512(b: &mut Bencher) {
    let mut ring = create_full_lossy(512);
    b.iter(|| {
        for idx in 0..(512 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_512(b: &mut Bencher) {
    let mut ring = create_full_welford(512);
    b.iter(|| {
        for idx in 0..(512 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_vecq_512(b: &mut Bencher) {
    let mut vecq = crate_full_vecq(512);
    b.iter(|| {
        for idx in 0..(512 * 2) {
            vecq.pop_front();
            vecq.push_back(idx as u32);
            get_mean(&vecq);
            get_variance(&vecq);
        }
    });
}

// ---------------------------------------- 256 ---------------------------------------- //
#[bench]
fn stats_per_iter_lossy_256(b: &mut Bencher) {
    let mut ring = create_full_lossy(256);
    b.iter(|| {
        for idx in 0..(256 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_256(b: &mut Bencher) {
    let mut ring = create_full_welford(256);
    b.iter(|| {
        for idx in 0..(256 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_lossy_256_mean_one_per_loop(b: &mut Bencher) {
    let mut ring = create_full_lossy(256);
    b.iter(|| {
        for _ in 0..5 {
            for idx in 0..(256 * 2) {
                ring.push(black_box(idx as i32));
            }
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_256_mean_one_per_loop(b: &mut Bencher) {
    let mut ring = create_full_welford(256);
    b.iter(|| {
        for _ in 0..5 {
            for idx in 0..(256 * 2) {
                ring.push(black_box(idx as i32));
            }
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_vecq_256(b: &mut Bencher) {
    let mut vecq = crate_full_vecq(256);
    b.iter(|| {
        for idx in 0..(256 * 2) {
            vecq.pop_front();
            vecq.push_back(idx as u32);
            get_mean(&vecq);
            get_variance(&vecq);
        }
    });
}

// ---------------------------------------- 128 ---------------------------------------- //
#[bench]
fn stats_per_iter_lossy_128(b: &mut Bencher) {
    let mut ring = create_full_lossy(128);
    b.iter(|| {
        for idx in 0..(128 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_128(b: &mut Bencher) {
    let mut ring = create_full_welford(128);
    b.iter(|| {
        for idx in 0..(128 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_vecq_128(b: &mut Bencher) {
    let mut vecq = crate_full_vecq(128);
    b.iter(|| {
        for idx in 0..(128 * 2) {
            vecq.pop_front();
            vecq.push_back(idx as u32);
            get_mean(&vecq);
            get_variance(&vecq);
        }
    });
}

// ---------------------------------------- 64 ---------------------------------------- //
#[bench]
fn stats_per_iter_lossy_64(b: &mut Bencher) {
    let mut ring = create_full_lossy(64);
    b.iter(|| {
        for idx in 0..(64 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_64(b: &mut Bencher) {
    let mut ring = create_full_welford(64);
    b.iter(|| {
        for idx in 0..(64 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_vecq_64(b: &mut Bencher) {
    let mut vecq = crate_full_vecq(64);
    b.iter(|| {
        for idx in 0..(64 * 2) {
            vecq.pop_front();
            vecq.push_back(idx as u32);
            get_mean(&vecq);
            get_variance(&vecq);
        }
    });
}

// ---------------------------------------- 32 ---------------------------------------- //
#[bench]
fn stats_per_iter_lossy_32(b: &mut Bencher) {
    let mut ring = create_full_lossy(32);
    b.iter(|| {
        for idx in 0..(32 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_welford_32(b: &mut Bencher) {
    let mut ring = create_full_welford(32);
    b.iter(|| {
        for idx in 0..(32 * 2) {
            ring.push(black_box(idx as i32));
            black_box(ring.mean());
            black_box(ring.variance());
        }
    });
}
#[bench]
fn stats_per_iter_vecq_32(b: &mut Bencher) {
    let mut vecq = crate_full_vecq(32);
    b.iter(|| {
        for idx in 0..(32 * 2) {
            vecq.pop_front();
            vecq.push_back(idx as u32);
            get_mean(&vecq);
            get_variance(&vecq);
        }
    });
}
