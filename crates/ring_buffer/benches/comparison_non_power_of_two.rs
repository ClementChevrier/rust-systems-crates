//! Same work on both sides at capacities that are not powers of two: the ring
//! rounds its storage up, the deque does not, so this measures what the
//! rounding costs.
#![feature(test)]

extern crate test;

use std::collections::VecDeque;
use test::{Bencher, black_box};

use ring_buffer::RingBuffer;

#[macro_use]
mod macros;
use macros::*;

// 1000  -> ring 1024: harmless rounding (+2.4 % memory).
// 1025  -> ring 2048: worst case, almost twice the memory asked for.
// 48000 -> ring 65536: 384 KiB asked, 512 KiB paid, exactly the kind of size
//          where rounding can change the cache level (the deque fits in L2,
//          the ring spills into L3).

bench_push_tail_bounded!(1000, push_back_c01000_ring, push_back_c01000_std);
bench_push_tail_bounded!(1025, push_back_c01025_ring, push_back_c01025_std);
bench_push_tail_bounded!(48000, push_back_c48000_ring, push_back_c48000_std);

bench_push_head_bounded!(1000, push_front_c01000_ring, push_front_c01000_std);
bench_push_head_bounded!(1025, push_front_c01025_ring, push_front_c01025_std);
bench_push_head_bounded!(48000, push_front_c48000_ring, push_front_c48000_std);

bench_rotate_front_to_back!(1000, rotate_fb_c01000_ring, rotate_fb_c01000_std);
bench_rotate_front_to_back!(1025, rotate_fb_c01025_ring, rotate_fb_c01025_std);
bench_rotate_front_to_back!(48000, rotate_fb_c48000_ring, rotate_fb_c48000_std);

bench_rotate_back_to_front!(1000, rotate_bf_c01000_ring, rotate_bf_c01000_std);
bench_rotate_back_to_front!(1025, rotate_bf_c01025_ring, rotate_bf_c01025_std);
bench_rotate_back_to_front!(48000, rotate_bf_c48000_ring, rotate_bf_c48000_std);

bench_iter_sum_wrapped!(1000, iter_sum_c01000_ring, iter_sum_c01000_std);
bench_iter_sum_wrapped!(1025, iter_sum_c01025_ring, iter_sum_c01025_std);
bench_iter_sum_wrapped!(48000, iter_sum_c48000_ring, iter_sum_c48000_std);
