//! Paths specific to the ring, with no std counterpart: `VecDeque` never refuses a push.
#![feature(test)]

extern crate test;

use test::{Bencher, black_box};

use ring_buffer::RingBuffer;

#[macro_use]
mod macros;
use macros::*;

// Refusal paths: no std counterpart, `VecDeque` never refuses.
#[bench]
fn try_push_full_reject_c01024_ring(b: &mut Bencher) {
    let mut buffer = filled_ring(1024);
    let mut rejected = 0u64;

    b.iter(|| {
        for value in 0..1024u64 {
            if buffer.try_push_tail(black_box(value)).is_err() {
                rejected = rejected.wrapping_add(1);
            }
        }
        black_box(rejected);
    });
}

#[bench]
fn try_push_pop_pair_c01024_ring(b: &mut Bencher) {
    let mut buffer = RingBuffer::try_new(1024).expect("benchmark capacity must be valid");
    let mut value = 0u64;
    b.bytes = lap_bytes(1024);

    b.iter(|| {
        for _ in 0..1024u64 {
            if buffer.try_push_tail(black_box(value)).is_err() {
                let popped = buffer.pop_head().expect("full buffer must contain a value");
                black_box(popped);
                assert!(buffer.try_push_tail(black_box(value)).is_ok());
            }
            value = value.wrapping_add(1);
        }
        black_box(buffer.len());
    });
}

// Control: the same sum over a flat Vec. If vec ~ std > ring, the suspect is
// the vectorisation of the loop over the ring's slices; if vec ~ ring, the
// deque has a specific advantage.
#[bench]
fn iter_sum_control_vec_c01024(b: &mut Bencher) {
    let values: Vec<u64> = (0..1024u64).collect();
    b.bytes = lap_bytes(1024);

    b.iter(|| {
        let sum = values
            .iter()
            .fold(0u64, |acc, value| acc.wrapping_add(*value));
        black_box(sum);
    });
}
