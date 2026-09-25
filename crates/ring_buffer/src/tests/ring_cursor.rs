#![allow(clippy::unwrap_used, clippy::panic)]

//! File written by IA!
//!
//! I don't really care about it the real test are made for ring_buffer as it's simplier
//! to test and verify!
//!
//! On top on that IA also generated test for ring_buffer and they were bad so I made them by hand
//! Only here to make clean!

use crate::error::*;

use super::*;

const CAP: usize = 8;

fn cursor() -> RingCursor {
    RingCursor::try_new(CAP).expect("CAP is a valid ring capacity")
}

fn full_cursor_from_back() -> RingCursor {
    let mut cursor = cursor();
    for _ in 0..CAP {
        cursor.push_tail_index();
        assert_valid(&cursor);
    }
    cursor
}

fn full_cursor_from_front() -> RingCursor {
    let mut cursor = cursor();
    for _ in 0..CAP {
        cursor.push_head_index();
        assert_valid(&cursor);
    }
    cursor
}

fn snapshot(cursor: &RingCursor) -> (usize, usize, usize) {
    (cursor.head, cursor.tail(), cursor.n_elem)
}

fn physical_sequence(cursor: &RingCursor) -> Vec<usize> {
    (0..cursor.len())
        .map(|logical_index| cursor.checked_physical_index(logical_index).unwrap())
        .collect()
}

fn assert_valid(cursor: &RingCursor) {
    assert!(cursor.cap.is_power_of_two());
    assert_eq!(cursor.mask, cursor.cap - 1);
    assert!(cursor.n_elem <= cursor.cap);
    assert!(cursor.head < cursor.cap);
    assert!(cursor.tail() < cursor.cap);

    for logical_index in 0..cursor.n_elem {
        let physical_index = cursor.checked_physical_index(logical_index).unwrap();
        assert!(physical_index < cursor.cap);
        assert_eq!(
            cursor.checked_physical_index(logical_index),
            Some(physical_index)
        );
    }

    assert_eq!(cursor.checked_physical_index(cursor.n_elem), None);
}

fn assert_sequence(cursor: &RingCursor, expected: &[usize]) {
    assert_valid(cursor);
    assert_eq!(cursor.len(), expected.len());
    assert_eq!(physical_sequence(cursor), expected);
}

mod init {
    use super::*;

    #[test]
    fn try_new_rejects_zero_capacity() {
        let result = RingCursor::try_new(0);

        assert!(matches!(result, Err(InitError::ZeroCapacity)));
    }

    #[test]
    fn try_new_rounds_capacity_to_next_power_of_two() {
        let cursor = RingCursor::try_new(15).expect("15 should round to 16");

        assert_eq!(cursor.real_size(), 16);
        assert_eq!(cursor.mask, 15);
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn try_new_keeps_power_of_two_capacity() {
        let cursor = RingCursor::try_new(32).expect("32 is a valid capacity");

        assert_eq!(cursor.asked_capacity(), 32);
        assert_eq!(cursor.mask, 31);
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn try_new_accepts_capacity_one() {
        let cursor = RingCursor::try_new(1).expect("1 is a valid capacity");

        assert_eq!(cursor.asked_capacity(), 1);
        assert_eq!(cursor.mask, 0);
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn try_new_rejects_capacity_overflow() {
        let result = RingCursor::try_new(usize::MAX);

        assert!(matches!(
            result,
            Err(InitError::CapacityOverflow {
                asked_cap: usize::MAX
            })
        ));
    }
}

mod physical_index {
    use super::*;

    #[test]
    fn checked_physical_index_rejects_empty_cursor_index_zero() {
        let cursor = cursor();

        assert_eq!(cursor.checked_physical_index(0), None);
    }

    #[test]
    fn checked_physical_index_accepts_existing_indices_only() {
        let mut cursor = cursor();

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_tail_index();

        assert_eq!(cursor.checked_physical_index(0), Some(0));
        assert_eq!(cursor.checked_physical_index(1), Some(1));
        assert_eq!(cursor.checked_physical_index(2), Some(2));
        assert_eq!(cursor.checked_physical_index(3), None);
    }
}

mod push_back {
    use super::*;

    #[test]
    fn push_back_on_empty_writes_index_zero() {
        let mut cursor = cursor();

        let written = cursor.push_tail_index();

        assert_eq!(written, 0);
        assert_eq!(cursor.head, 0);
        assert_eq!(cursor.tail(), 1);
        assert_sequence(&cursor, &[0]);
    }

    #[test]
    fn push_back_appends_in_physical_order_until_wrap() {
        let mut cursor = cursor();

        let first = cursor.push_tail_index();
        let second = cursor.push_tail_index();
        let third = cursor.push_tail_index();

        assert_eq!([first, second, third], [0, 1, 2]);
        assert_eq!(cursor.head, 0);
        assert_eq!(cursor.tail(), 3);
        assert_sequence(&cursor, &[0, 1, 2]);
    }

    #[test]
    fn push_back_wraps_after_front_pops() {
        let mut cursor = cursor();

        for _ in 0..6 {
            cursor.push_tail_index();
        }

        assert_eq!(cursor.pop_head_index(), Some(0));
        assert_eq!(cursor.pop_head_index(), Some(1));
        assert_eq!(cursor.pop_head_index(), Some(2));
        assert_eq!(cursor.pop_head_index(), Some(3));

        assert_sequence(&cursor, &[4, 5]);

        assert_eq!(cursor.push_tail_index(), 6);
        assert_eq!(cursor.push_tail_index(), 7);
        assert_eq!(cursor.push_tail_index(), 0);
        assert_eq!(cursor.push_tail_index(), 1);

        assert_eq!(cursor.head, 4);
        assert_eq!(cursor.tail(), 2);
        assert_sequence(&cursor, &[4, 5, 6, 7, 0, 1]);
    }
}

mod push_front {
    use super::*;

    #[test]
    fn push_front_on_empty_writes_last_index() {
        let mut cursor = cursor();

        let written = cursor.push_head_index();

        assert_eq!(written, CAP - 1);
        assert_eq!(cursor.head, CAP - 1);
        assert_eq!(cursor.tail(), 0);
        assert_sequence(&cursor, &[CAP - 1]);
    }

    #[test]
    fn push_front_prepends_in_reverse_physical_order() {
        let mut cursor = cursor();

        let first = cursor.push_head_index();
        let second = cursor.push_head_index();
        let third = cursor.push_head_index();

        assert_eq!([first, second, third], [7, 6, 5]);
        assert_eq!(cursor.head, 5);
        assert_eq!(cursor.tail(), 0);
        assert_sequence(&cursor, &[5, 6, 7]);
    }

    #[test]
    fn mixed_push_front_and_push_back_preserve_logical_order() {
        let mut cursor = cursor();

        assert_eq!(cursor.push_tail_index(), 0);
        assert_eq!(cursor.push_tail_index(), 1);
        assert_eq!(cursor.push_head_index(), 7);
        assert_eq!(cursor.push_tail_index(), 2);

        assert_eq!(cursor.head, 7);
        assert_eq!(cursor.tail(), 3);
        assert_sequence(&cursor, &[7, 0, 1, 2]);
    }
}

mod try_push {
    use super::*;

    #[test]
    fn try_push_back_returns_written_index_when_not_full() {
        let mut cursor = cursor();

        assert_eq!(cursor.try_push_tail_index(), Some(0));
        assert_sequence(&cursor, &[0]);
    }

    #[test]
    fn try_push_front_returns_written_index_when_not_full() {
        let mut cursor = cursor();

        assert_eq!(cursor.try_push_head_index(), Some(7));
        assert_sequence(&cursor, &[7]);
    }

    #[test]
    fn try_push_back_on_full_cursor_returns_none_without_mutating() {
        let mut cursor = full_cursor_from_back();
        let before = snapshot(&cursor);
        let before_sequence = physical_sequence(&cursor);

        let result = cursor.try_push_tail_index();

        assert_eq!(result, None);
        assert_eq!(snapshot(&cursor), before);
        assert_eq!(physical_sequence(&cursor), before_sequence);
        assert_valid(&cursor);
    }

    #[test]
    fn try_push_front_on_full_cursor_returns_none_without_mutating() {
        let mut cursor = full_cursor_from_back();
        let before = snapshot(&cursor);
        let before_sequence = physical_sequence(&cursor);

        let result = cursor.try_push_head_index();

        assert_eq!(result, None);
        assert_eq!(snapshot(&cursor), before);
        assert_eq!(physical_sequence(&cursor), before_sequence);
        assert_valid(&cursor);
    }
}

mod overwrite_when_full {
    use super::*;

    #[test]
    fn push_back_on_full_cursor_overwrites_front() {
        let mut cursor = full_cursor_from_back();

        assert_sequence(&cursor, &[0, 1, 2, 3, 4, 5, 6, 7]);

        let written = cursor.push_tail_index();

        assert_eq!(written, 0);
        assert_eq!(cursor.len(), CAP);
        assert_eq!(cursor.head, 1);
        assert_eq!(cursor.tail(), 1);
        assert_sequence(&cursor, &[1, 2, 3, 4, 5, 6, 7, 0]);
    }

    #[test]
    fn push_front_on_full_cursor_overwrites_back() {
        let mut cursor = full_cursor_from_back();

        assert_sequence(&cursor, &[0, 1, 2, 3, 4, 5, 6, 7]);

        let written = cursor.push_head_index();

        assert_eq!(written, 7);
        assert_eq!(cursor.len(), CAP);
        assert_eq!(cursor.head, 7);
        assert_eq!(cursor.tail(), 7);
        assert_sequence(&cursor, &[7, 0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn repeated_push_back_on_full_cursor_keeps_capacity_and_moves_window() {
        let mut cursor = full_cursor_from_back();

        assert_eq!(cursor.push_tail_index(), 0);
        assert_eq!(cursor.push_tail_index(), 1);
        assert_eq!(cursor.push_tail_index(), 2);

        assert_eq!(cursor.len(), CAP);
        assert_eq!(cursor.head, 3);
        assert_eq!(cursor.tail(), 3);
        assert_sequence(&cursor, &[3, 4, 5, 6, 7, 0, 1, 2]);
    }

    #[test]
    fn repeated_push_front_on_full_cursor_keeps_capacity_and_moves_window() {
        let mut cursor = full_cursor_from_back();

        assert_eq!(cursor.push_head_index(), 7);
        assert_eq!(cursor.push_head_index(), 6);
        assert_eq!(cursor.push_head_index(), 5);

        assert_eq!(cursor.len(), CAP);
        assert_eq!(cursor.head, 5);
        assert_eq!(cursor.tail(), 5);
        assert_sequence(&cursor, &[5, 6, 7, 0, 1, 2, 3, 4]);
    }
}

mod pop_front {
    use super::*;

    #[test]
    fn pop_front_on_empty_cursor_returns_none_without_mutating() {
        let mut cursor = cursor();
        let before = snapshot(&cursor);

        let popped = cursor.pop_head_index();

        assert_eq!(popped, None);
        assert_eq!(snapshot(&cursor), before);
        assert_valid(&cursor);
    }

    #[test]
    fn push_back_then_pop_front_returns_same_index() {
        let mut cursor = cursor();

        let written = cursor.push_tail_index();
        let popped = cursor.pop_head_index();

        assert_eq!(popped, Some(written));
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn push_front_then_pop_front_returns_same_index() {
        let mut cursor = cursor();

        let written = cursor.push_head_index();
        let popped = cursor.pop_head_index();

        assert_eq!(popped, Some(written));
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn pop_front_removes_logical_front() {
        let mut cursor = cursor();

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_head_index();
        cursor.push_tail_index();

        assert_sequence(&cursor, &[7, 0, 1, 2]);

        assert_eq!(cursor.pop_head_index(), Some(7));
        assert_sequence(&cursor, &[0, 1, 2]);

        assert_eq!(cursor.pop_head_index(), Some(0));
        assert_sequence(&cursor, &[1, 2]);
    }

    #[test]
    fn pop_front_until_empty_preserves_fifo_order() {
        let mut cursor = full_cursor_from_back();

        for expected in 0..CAP {
            assert_eq!(cursor.pop_head_index(), Some(expected));
            assert_valid(&cursor);
        }

        assert_eq!(cursor.pop_head_index(), None);
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }
}

mod pop_back {
    use super::*;

    #[test]
    fn pop_back_on_empty_cursor_returns_none_without_mutating() {
        let mut cursor = cursor();
        let before = snapshot(&cursor);

        let popped = cursor.pop_tail_index();

        assert_eq!(popped, None);
        assert_eq!(snapshot(&cursor), before);
        assert_valid(&cursor);
    }

    #[test]
    fn push_back_then_pop_back_returns_same_index() {
        let mut cursor = cursor();

        let written = cursor.push_tail_index();
        let popped = cursor.pop_tail_index();

        assert_eq!(popped, Some(written));
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn push_front_then_pop_back_returns_same_index() {
        let mut cursor = cursor();

        let written = cursor.push_head_index();
        let popped = cursor.pop_tail_index();

        assert_eq!(popped, Some(written));
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn pop_back_removes_logical_back() {
        let mut cursor = cursor();

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_head_index();
        cursor.push_tail_index();

        assert_sequence(&cursor, &[7, 0, 1, 2]);

        assert_eq!(cursor.pop_tail_index(), Some(2));
        assert_sequence(&cursor, &[7, 0, 1]);

        assert_eq!(cursor.pop_tail_index(), Some(1));
        assert_sequence(&cursor, &[7, 0]);
    }

    #[test]
    fn pop_back_until_empty_preserves_lifo_order() {
        let mut cursor = full_cursor_from_back();

        for expected in (0..CAP).rev() {
            assert_eq!(cursor.pop_tail_index(), Some(expected));
            assert_valid(&cursor);
        }

        assert_eq!(cursor.pop_tail_index(), None);
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }
}

mod full_layout {
    use super::*;

    #[test]
    fn full_cursor_from_back_has_expected_layout() {
        let cursor = full_cursor_from_back();

        assert!(cursor.is_full());
        assert_eq!(cursor.len(), CAP);
        assert_eq!(cursor.head, 0);
        assert_eq!(cursor.tail(), 0);
        assert_sequence(&cursor, &[0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn full_cursor_from_front_has_expected_layout() {
        let cursor = full_cursor_from_front();

        assert!(cursor.is_full());
        assert_eq!(cursor.len(), CAP);
        assert_eq!(cursor.head, 0);
        assert_eq!(cursor.tail(), 0);
        assert_sequence(&cursor, &[0, 1, 2, 3, 4, 5, 6, 7]);
    }
}

mod as_slices {
    use super::*;

    #[test]
    fn as_slices_empty_returns_two_empty_slices() {
        let cursor = cursor();
        let values = [0, 1, 2, 3, 4, 5, 6, 7];

        let (first, second) = cursor.as_slices(&values);

        assert_eq!(first, &[]);
        assert_eq!(second, &[]);
        assert_valid(&cursor);
    }

    #[test]
    fn as_slices_contiguous_returns_single_slice() {
        let mut cursor = cursor();
        let values = [0, 1, 2, 3, 4, 5, 6, 7];

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_tail_index();

        let (first, second) = cursor.as_slices(&values);

        assert_eq!(first, &[0, 1, 2]);
        assert_eq!(second, &[]);
        assert_sequence(&cursor, &[0, 1, 2]);
    }

    #[test]
    fn as_slices_wrapped_returns_right_then_left_slice() {
        let mut cursor = cursor();
        let values = [0, 1, 2, 3, 4, 5, 6, 7];

        for _ in 0..6 {
            cursor.push_tail_index();
        }

        for _ in 0..4 {
            cursor.pop_head_index();
        }

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_tail_index();

        assert_sequence(&cursor, &[4, 5, 6, 7, 0, 1]);

        let (first, second) = cursor.as_slices(&values);

        assert_eq!(first, &[4, 5, 6, 7]);
        assert_eq!(second, &[0, 1]);
    }

    #[test]
    fn as_slices_full_contiguous_returns_single_full_slice() {
        let cursor = full_cursor_from_back();
        let values = [0, 1, 2, 3, 4, 5, 6, 7];

        let (first, second) = cursor.as_slices(&values);

        assert_eq!(first, &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(second, &[]);
    }

    #[test]
    fn as_slices_full_wrapped_after_push_back_overwrite() {
        let mut cursor = full_cursor_from_back();
        let values = [0, 1, 2, 3, 4, 5, 6, 7];

        cursor.push_tail_index();

        assert_sequence(&cursor, &[1, 2, 3, 4, 5, 6, 7, 0]);

        let (first, second) = cursor.as_slices(&values);

        assert_eq!(first, &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(second, &[0]);
    }

    #[test]
    fn as_slices_full_wrapped_after_push_front_overwrite() {
        let mut cursor = full_cursor_from_back();
        let values = [0, 1, 2, 3, 4, 5, 6, 7];

        cursor.push_head_index();

        assert_sequence(&cursor, &[7, 0, 1, 2, 3, 4, 5, 6]);

        let (first, second) = cursor.as_slices(&values);

        assert_eq!(first, &[7]);
        assert_eq!(second, &[0, 1, 2, 3, 4, 5, 6]);
    }
}

mod capacity_one {
    use super::*;

    #[test]
    fn capacity_one_push_back_pop_front_roundtrip() {
        let mut cursor = RingCursor::try_new(1).expect("capacity must be valid");

        let written = cursor.push_tail_index();

        assert_eq!(written, 0);
        assert!(cursor.is_full());
        assert_sequence(&cursor, &[0]);

        assert_eq!(cursor.pop_head_index(), Some(0));
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn capacity_one_push_front_pop_back_roundtrip() {
        let mut cursor = RingCursor::try_new(1).expect("capacity must be valid");

        let written = cursor.push_head_index();

        assert_eq!(written, 0);
        assert!(cursor.is_full());
        assert_sequence(&cursor, &[0]);

        assert_eq!(cursor.pop_tail_index(), Some(0));
        assert_eq!(cursor.len(), 0);
        assert_valid(&cursor);
    }

    #[test]
    fn capacity_one_try_push_refuses_when_full_without_mutating() {
        let mut cursor = RingCursor::try_new(1).expect("capacity must be valid");

        assert_eq!(cursor.push_tail_index(), 0);

        let before = snapshot(&cursor);

        assert_eq!(cursor.try_push_tail_index(), None);
        assert_eq!(cursor.try_push_head_index(), None);
        assert_eq!(snapshot(&cursor), before);
        assert_sequence(&cursor, &[0]);
    }

    #[test]
    fn capacity_one_push_overwrites_same_slot_when_full() {
        let mut cursor = RingCursor::try_new(1).expect("capacity must be valid");

        assert_eq!(cursor.push_tail_index(), 0);
        assert_eq!(cursor.push_tail_index(), 0);
        assert_sequence(&cursor, &[0]);

        assert_eq!(cursor.push_head_index(), 0);
        assert_sequence(&cursor, &[0]);
    }
}

mod mixed_sequences {
    use super::*;

    #[test]
    fn mixed_operations_keep_cursor_valid() {
        let mut cursor = cursor();

        assert_valid(&cursor);

        cursor.push_tail_index();
        assert_valid(&cursor);

        cursor.push_head_index();
        assert_valid(&cursor);

        cursor.push_tail_index();
        assert_valid(&cursor);

        cursor.pop_head_index();
        assert_valid(&cursor);

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_head_index();
        assert_valid(&cursor);

        cursor.pop_tail_index();
        cursor.pop_head_index();
        assert_valid(&cursor);
    }

    #[test]
    fn mixed_operations_preserve_expected_sequence() {
        let mut cursor = cursor();

        cursor.push_tail_index();
        cursor.push_tail_index();
        cursor.push_head_index();
        cursor.push_tail_index();

        assert_sequence(&cursor, &[7, 0, 1, 2]);

        assert_eq!(cursor.pop_head_index(), Some(7));
        assert_sequence(&cursor, &[0, 1, 2]);

        assert_eq!(cursor.pop_tail_index(), Some(2));
        assert_sequence(&cursor, &[0, 1]);

        assert_eq!(cursor.push_head_index(), 7);
        assert_eq!(cursor.push_tail_index(), 2);
        assert_sequence(&cursor, &[7, 0, 1, 2]);
    }
}

mod skip {
    use super::*;

    #[test]
    fn skip_zero_is_a_noop() {
        let mut cursor = cursor();
        for _ in 0..3 {
            cursor.push_tail_index();
        }
        let before = snapshot(&cursor);

        cursor.skip(0);

        assert_eq!(snapshot(&cursor), before);
        assert_sequence(&cursor, &[0, 1, 2]);
    }

    #[test]
    fn skip_partial_advances_head_and_shrinks_len() {
        let mut cursor = cursor();
        for _ in 0..5 {
            cursor.push_tail_index();
        } // head=0, tail=5, len=5

        cursor.skip(2);

        assert_eq!(cursor.head, 2);
        assert_eq!(cursor.len(), 3);
        assert_sequence(&cursor, &[2, 3, 4]);
    }

    #[test]
    fn skip_all_empties_the_cursor() {
        let mut cursor = cursor();
        for _ in 0..5 {
            cursor.push_tail_index();
        }

        let n = cursor.len();
        cursor.skip(n);

        assert!(cursor.is_empty());
        assert_eq!(cursor.head, 5);
        assert_eq!(cursor.tail(), 5);
        assert_valid(&cursor);
    }

    #[test]
    fn skip_masks_head_on_wraparound() {
        // Wrapped window: head=4, tail=2, len=6, physical sequence [4,5,6,7,0,1].
        let mut cursor = cursor();
        for _ in 0..6 {
            cursor.push_tail_index();
        }
        for _ in 0..4 {
            cursor.pop_head_index();
        }
        for _ in 0..4 {
            cursor.push_tail_index();
        }
        assert_sequence(&cursor, &[4, 5, 6, 7, 0, 1]);

        // head = (4+5) & 7 = 1. Without the mask, head=9 > cap and assert_valid would fail.
        cursor.skip(5);

        assert_eq!(cursor.head, 1);
        assert_eq!(cursor.len(), 1);
        assert_sequence(&cursor, &[1]);
    }

    #[test]
    fn push_pop_stay_consistent_after_a_skip() {
        let mut cursor = cursor();
        for _ in 0..5 {
            cursor.push_tail_index();
        }
        cursor.skip(3); // head=3, tail=5, len=2, sequence [3,4]

        cursor.push_tail_index();
        assert_sequence(&cursor, &[3, 4, 5]);
        assert_eq!(cursor.pop_head_index(), Some(3));
        assert_sequence(&cursor, &[4, 5]);
    }
}
