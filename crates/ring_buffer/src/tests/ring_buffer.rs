#![allow(clippy::unwrap_used, clippy::panic)]

use crate::error::*;

use super::*;

fn u64_buffer_with_capacity(requested_capacity: usize) -> RingBuffer<u64> {
    RingBuffer::try_new(requested_capacity).expect("requested capacity must be valid")
}

fn logical_values(buffer: &RingBuffer<u64>) -> Vec<u64> {
    let (first, second) = buffer.as_slices();

    first.iter().chain(second.iter()).copied().collect()
}

fn assert_valid(buffer: &RingBuffer<u64>) {
    assert!(buffer.cursor.real_size().is_power_of_two());
    assert_eq!(buffer.cursor.mask, buffer.cursor.real_size() - 1);
    assert_eq!(buffer.inner.len(), buffer.cursor.real_size());
    assert!(buffer.len() <= buffer.cursor.real_size());
    assert_eq!(logical_values(buffer).len(), buffer.len());
}

fn assert_sequence(buffer: &RingBuffer<u64>, expected: &[u64]) {
    assert_valid(buffer);
    assert_eq!(buffer.len(), expected.len());
    assert_eq!(logical_values(buffer), expected);

    for (logical_index, expected_value) in expected.iter().enumerate() {
        assert_eq!(buffer.peek(logical_index), Some(expected_value));
    }
    assert_eq!(buffer.get(expected.len()), None);
}

mod behaviour {
    use super::*;

    #[test]
    fn init_fail() {
        assert!(matches!(
            RingBuffer::<u64>::try_new(0).err(),
            Some(InitError::ZeroCapacity)
        ));

        // Max number until next power of 2 returns None;
        let last_valid_cap = 1usize << (usize::BITS - 1);
        // Will provoque an Oom as I try to alloc too much mem
        // assert!(RingBuffer::<u64>::try_new(last_valid_cap).is_ok());

        let overflow_value = last_valid_cap + 1;
        assert!(matches!(
            RingBuffer::<u64>::try_new(overflow_value).err(),
            Some(InitError::CapacityOverflow {
                asked_cap: _overflow_value
            })
        ));
    }

    #[test]
    fn test_wrap_push_try_back() {
        let mut buffer = u64_buffer_with_capacity(6);

        for value in 0..6 {
            assert!(buffer.try_push_tail(value as u64).is_ok());
        }

        for value in 6..8 {
            assert!(buffer.try_push_tail(value as u64).is_err());
            buffer.push_tail(value as u64);
        }

        // println!("{buffer:?}");
        assert_eq!(buffer.get(0), Some(2));
        assert_eq!(buffer.get(7), None);
        assert_sequence(&buffer, &[2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_wrap_push_try_front() {
        let mut buffer = u64_buffer_with_capacity(6);

        for value in 0..6 {
            assert!(buffer.try_push_head(value as u64).is_ok());
        }

        for value in 6..8 {
            assert!(buffer.try_push_head(value as u64).is_err());
            buffer.push_head(value as u64);
        }

        // println!("{buffer}");
        assert_eq!(buffer.get(0), Some(7));
        assert_eq!(buffer.get(7), None);
        assert_sequence(&buffer, &[7, 6, 5, 4, 3, 2]);
    }

    #[test]
    fn test_wrap_push_try_back_evicting() {
        let mut buffer = u64_buffer_with_capacity(6);

        for value in 0..6 {
            assert!(buffer.try_push_tail(value).is_ok());
        }

        for value in 6..8 {
            assert!(buffer.try_push_tail(value).is_err());
            assert_eq!(buffer.push_tail_evicting(value).unwrap(), value - 6);
        }

        // println!("{buffer:?}");
        assert_eq!(buffer.get(0), Some(2));
        assert_eq!(buffer.get(7), None);
        assert_sequence(&buffer, &[2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_wrap_push_try_front_evicting() {
        let mut buffer = u64_buffer_with_capacity(6);

        for value in 0..6 {
            assert!(buffer.try_push_head(value).is_ok());
        }

        for value in 6..8 {
            assert!(buffer.try_push_head(value).is_err());
            assert_eq!(buffer.push_head_evicting(value).unwrap(), value - 6);
        }

        // println!("{buffer:?}");
        assert_eq!(buffer.get(0), Some(7));
        assert_eq!(buffer.get(7), None);
        assert_sequence(&buffer, &[7, 6, 5, 4, 3, 2]);
    }

    #[test]
    /// Same as above but try push_back should failed!
    fn test_wrap_push_try_back_overflow() {
        let mut buffer = u64_buffer_with_capacity(6);

        for value in 0..6 {
            assert!(buffer.try_push_tail(value as u64).is_ok());
        }

        for value in 6..14 {
            assert!(buffer.try_push_tail(value as u64).is_err());
            buffer.push_tail(value as u64)
        }

        // println!("{buffer}");
        assert_eq!(buffer.get(7), None);
        assert_eq!(buffer.get(0), Some(8));
        assert_sequence(&buffer, &[8, 9, 10, 11, 12, 13]);
    }

    #[test]
    /// Same as above but try push_front should failed!
    fn test_wrap_push_try_front_overflow() {
        let mut buffer = u64_buffer_with_capacity(6);

        for value in 0..6 {
            assert!(buffer.try_push_head(value as u64).is_ok());
        }

        for value in 6..14 {
            assert!(buffer.try_push_head(value as u64).is_err());
            buffer.push_head(value as u64)
        }

        // println!("{buffer}");
        assert_eq!(buffer.get(0), Some(13));
        assert_eq!(buffer.get(7), None);
        assert_sequence(&buffer, &[13, 12, 11, 10, 9, 8]);
    }

    #[test]
    fn iter_all_combinations() {
        let mut buffer = u64_buffer_with_capacity(8);

        // println!("{buffer}");
        for value in 0..32 {
            buffer.push_tail(value);

            // Check Iter
            // println!("{buffer}");
            assert_eq!(
                buffer.iter_head_to_tail().cloned().collect::<Vec<u64>>(),
                ((value.saturating_sub(7)..=value)
                    .into_iter()
                    .collect::<Vec<u64>>())
            );
        }
    }

    #[test]
    fn utils_get_peek_peekmut() {
        let mut buffer = u64_buffer_with_capacity(8);
        for value in 0..8 {
            assert!(buffer.try_push_tail(value as u64).is_ok());
        }

        assert_eq!(buffer.get(7), Some(7));
        assert_eq!(buffer.peek(7), Some(&7));
        assert_eq!(buffer.peek_mut(7), Some(&mut 7));
    }

    #[test]
    fn pop_simple_push_back() {
        let mut buffer = u64_buffer_with_capacity(8);
        for value in 0..8 {
            assert!(buffer.try_push_tail(value as u64).is_ok());
        }

        println!("{buffer}");
        assert_eq!(buffer.pop_head(), Some(0));
        assert_eq!(buffer.pop_tail(), Some(7));
        assert_sequence(&buffer, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(
            buffer.iter_head_to_tail().cloned().collect::<Vec<u64>>(),
            (1..7).into_iter().collect::<Vec<u64>>()
        );
    }

    #[test]
    fn pop_simple_push_front() {
        let mut buffer = u64_buffer_with_capacity(8);
        for value in 0..8 {
            assert!(buffer.try_push_head(value as u64).is_ok());
        }

        println!("{buffer}");
        assert_eq!(buffer.pop_head(), Some(7));
        assert_eq!(buffer.pop_tail(), Some(0));
        assert_sequence(&buffer, &[6, 5, 4, 3, 2, 1]);
        assert_eq!(
            buffer.iter_head_to_tail().cloned().collect::<Vec<u64>>(),
            (1..7).into_iter().rev().collect::<Vec<u64>>()
        );
    }

    #[test]
    fn drain_full() {
        let mut buffer = u64_buffer_with_capacity(6);
        for value in 0..8 {
            buffer.push_tail(value);
        }

        let drain = buffer.drain_from_head();
        assert_eq!(drain.len(), 6); /* .len() is available due to ExactSizeIterator trait! */
        assert_eq!(
            drain.collect::<Vec<u64>>(),
            (2..8).into_iter().collect::<Vec<u64>>()
        );

        assert!(buffer.is_empty())
    }

    #[test]
    fn drain_partial_with_drop() {
        let mut buffer = u64_buffer_with_capacity(6);
        for value in 0..8 {
            buffer.push_tail(value);
        }

        let mut drain = buffer.drain_from_head();
        for i in 2..5 {
            assert_eq!(drain.next().unwrap(), i)
        }

        // Drain automically drop here.

        assert_eq!(buffer.len(), 3);
        assert_sequence(&buffer, &[5, 6, 7]);
    }

    #[test]
    fn clear_works() {
        let mut buffer = u64_buffer_with_capacity(6);
        for value in 0..8 {
            buffer.push_tail(value);
        }

        buffer.clear();
        assert!(buffer.is_empty())
    }

    #[test]
    fn clone() {
        let mut buffer = u64_buffer_with_capacity(6);
        for value in 0..8 {
            buffer.push_tail(value);
        }

        let new_buffer = buffer.clone();
        assert_eq!(
            new_buffer.iter_head_to_tail().collect::<Vec<&u64>>(),
            buffer.iter_head_to_tail().collect::<Vec<&u64>>()
        )
    }
}
