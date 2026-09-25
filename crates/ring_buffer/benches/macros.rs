//! Macros and helpers shared by the bench targets. Not a target itself: each
//! file in `benches/` includes it with `mod macros;` (hence `autobenches = false`).

// Each bench target uses a different subset of these helpers.
#![allow(dead_code)]

use std::collections::VecDeque;

use ring_buffer::RingBuffer;

pub(crate) const fn lap_bytes(capacity: usize) -> u64 {
    (capacity * size_of::<u64>()) as u64
}

pub(crate) fn filled_ring(capacity: usize) -> RingBuffer<u64> {
    let mut buffer = RingBuffer::try_new(capacity).expect("benchmark capacity must be valid");
    for value in 0..capacity as u64 {
        buffer.push_tail(value);
    }
    buffer
}

pub(crate) fn filled_deque(capacity: usize) -> VecDeque<u64> {
    let mut deque = VecDeque::with_capacity(capacity);
    for value in 0..capacity as u64 {
        deque.push_back(value);
    }
    deque
}

pub(crate) fn wrapped_ring(requested_capacity: usize) -> RingBuffer<u64> {
    let mut buffer =
        RingBuffer::try_new(requested_capacity).expect("benchmark capacity must be valid");
    let real_capacity = buffer.asked_capacity();
    for value in 0..(real_capacity + real_capacity / 2) as u64 {
        buffer.push_tail(value);
    }
    buffer
}

pub(crate) fn wrapped_deque(capacity: usize) -> VecDeque<u64> {
    let mut deque = VecDeque::with_capacity(capacity);
    for value in 0..(capacity + capacity / 2) as u64 {
        if deque.len() == capacity {
            deque.pop_front();
        }
        deque.push_back(value);
    }
    deque
}

/// Bounded push: native overwrite on the ring side, pop + push on the std side.
#[allow(unused_macros)]
macro_rules! bench_push_tail_bounded {
    ($cap:expr, $ring:ident, $std:ident) => {
        #[bench]
        fn $ring(b: &mut Bencher) {
            let mut buffer = RingBuffer::try_new($cap).expect("benchmark capacity must be valid");
            let mut value = 0u64;
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    buffer.push_tail(black_box(value));
                    value = value.wrapping_add(1);
                }
                black_box(buffer.len());
            });
        }

        #[bench]
        fn $std(b: &mut Bencher) {
            let mut deque = VecDeque::with_capacity($cap);
            let mut value = 0u64;
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    if deque.len() == $cap {
                        deque.pop_front();
                    }
                    deque.push_back(black_box(value));
                    value = value.wrapping_add(1);
                }
                black_box(deque.len());
            });
        }
    };
}

/// Sequential read over a wrapped buffer (two slices).
#[allow(unused_macros)]
macro_rules! bench_iter_sum_wrapped {
    ($cap:expr, $ring:ident, $std:ident) => {
        #[bench]
        fn $ring(b: &mut Bencher) {
            let buffer = wrapped_ring($cap);
            b.bytes = (buffer.len() * size_of::<u64>()) as u64;

            b.iter(|| {
                let (first, second) = buffer.as_slices();
                let sum = first
                    .iter()
                    .chain(second.iter())
                    .fold(0u64, |acc, value| acc.wrapping_add(*value));
                black_box(sum);
            });
        }

        #[bench]
        fn $std(b: &mut Bencher) {
            let deque = wrapped_deque($cap);
            b.bytes = (deque.len() * size_of::<u64>()) as u64;

            b.iter(|| {
                let (first, second) = deque.as_slices();
                let sum = first
                    .iter()
                    .chain(second.iter())
                    .fold(0u64, |acc, value| acc.wrapping_add(*value));
                black_box(sum);
            });
        }
    };
}

#[allow(unused_macros)]
macro_rules! bench_push_head_bounded {
    ($cap:expr, $ring:ident, $std:ident) => {
        #[bench]
        fn $ring(b: &mut Bencher) {
            let mut buffer = RingBuffer::try_new($cap).expect("benchmark capacity must be valid");
            let mut value = 0u64;
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    buffer.push_head(black_box(value));
                    value = value.wrapping_add(1);
                }
                black_box(buffer.len());
            });
        }

        #[bench]
        fn $std(b: &mut Bencher) {
            let mut deque = VecDeque::with_capacity($cap);
            let mut value = 0u64;
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    if deque.len() == $cap {
                        deque.pop_back();
                    }
                    deque.push_front(black_box(value));
                    value = value.wrapping_add(1);
                }
                black_box(deque.len());
            });
        }
    };
}

/// Rotation: strictly the same work on both sides.
#[allow(unused_macros)]
macro_rules! bench_rotate_front_to_back {
    ($cap:expr, $ring:ident, $std:ident) => {
        #[bench]
        fn $ring(b: &mut Bencher) {
            let mut buffer = filled_ring($cap);
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    let value = buffer
                        .pop_head()
                        .expect("benchmark buffer must stay non-empty");
                    buffer.push_tail(black_box(value.wrapping_add(1)));
                }
                black_box(buffer.len());
            });
        }

        #[bench]
        fn $std(b: &mut Bencher) {
            let mut deque = filled_deque($cap);
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    let value = deque
                        .pop_front()
                        .expect("benchmark deque must stay non-empty");
                    deque.push_back(black_box(value.wrapping_add(1)));
                }
                black_box(deque.len());
            });
        }
    };
}

#[allow(unused_macros)]
macro_rules! bench_rotate_back_to_front {
    ($cap:expr, $ring:ident, $std:ident) => {
        #[bench]
        fn $ring(b: &mut Bencher) {
            let mut buffer = filled_ring($cap);
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    let value = buffer
                        .pop_tail()
                        .expect("benchmark buffer must stay non-empty");
                    buffer.push_head(black_box(value.wrapping_add(1)));
                }
                black_box(buffer.len());
            });
        }

        #[bench]
        fn $std(b: &mut Bencher) {
            let mut deque = filled_deque($cap);
            b.bytes = lap_bytes($cap);

            b.iter(|| {
                for _ in 0..$cap {
                    let value = deque
                        .pop_front()
                        .expect("benchmark deque must stay non-empty");
                    deque.push_front(black_box(value.wrapping_add(1)));
                }
                black_box(deque.len());
            });
        }
    };
}
