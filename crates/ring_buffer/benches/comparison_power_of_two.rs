//! Same work on both sides: power-of-two capacities, so the ring and the deque
//! handle exactly the same number of slots.
#![feature(test)]

extern crate test;

use std::collections::VecDeque;
use test::{Bencher, black_box};

use ring_buffer::RingBuffer;

#[macro_use]
mod macros;
use macros::*;

bench_push_tail_bounded!(64, push_back_c00064_ring, push_back_c00064_std);
bench_push_tail_bounded!(1024, push_back_c01024_ring, push_back_c01024_std);
bench_push_tail_bounded!(65536, push_back_c65536_ring, push_back_c65536_std);

bench_push_head_bounded!(64, push_front_c00064_ring, push_front_c00064_std);
bench_push_head_bounded!(1024, push_front_c01024_ring, push_front_c01024_std);
bench_push_head_bounded!(65536, push_front_c65536_ring, push_front_c65536_std);

bench_rotate_front_to_back!(64, rotate_fb_c00064_ring, rotate_fb_c00064_std);
bench_rotate_front_to_back!(1024, rotate_fb_c01024_ring, rotate_fb_c01024_std);
bench_rotate_front_to_back!(65536, rotate_fb_c65536_ring, rotate_fb_c65536_std);

bench_rotate_back_to_front!(64, rotate_bf_c00064_ring, rotate_bf_c00064_std);
bench_rotate_back_to_front!(1024, rotate_bf_c01024_ring, rotate_bf_c01024_std);
bench_rotate_back_to_front!(65536, rotate_bf_c65536_ring, rotate_bf_c65536_std);

bench_iter_sum_wrapped!(64, iter_sum_c00064_ring, iter_sum_c00064_std);
bench_iter_sum_wrapped!(1024, iter_sum_c01024_ring, iter_sum_c01024_std);
bench_iter_sum_wrapped!(65536, iter_sum_c65536_ring, iter_sum_c65536_std);
