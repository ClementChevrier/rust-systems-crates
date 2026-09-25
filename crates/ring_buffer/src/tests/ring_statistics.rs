#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

type DefaultRing = RingStatistics<u32, LossyAccumulator<u128>>;

fn create_new_buffer(windows: usize) -> (DefaultRing, Vec<u32>) {
    let mut ring = RingStatistics::try_new(windows, LossyAccumulator::default()).unwrap();
    let mut vec = Vec::with_capacity(windows);

    for i in 0..windows {
        ring.push(i as u32);
        vec.push(i as u32);
    }

    (ring, vec)
}

fn assert_same_elem(ring: &DefaultRing, vec: &[u32]) {
    for i in 0..ring.len() {
        assert_eq!(ring.peek(i), vec.get(vec.len() - 1 - i))
    }
}

fn get_mean(data: &[u32]) -> f64 {
    data.iter().sum::<u32>() as f64 / data.len() as f64
}

fn get_variance(data: &[u32]) -> f64 {
    let mean = data.iter().map(|&v| v as f64).sum::<f64>() / data.len() as f64;
    data.iter()
        .map(|&value| {
            let diff = mean - value as f64;
            diff * diff
        })
        .sum::<f64>()
        / data.len() as f64
}

#[test]
fn ring_vec_identical_structure() {
    let (ring, vec) = create_new_buffer(8);
    assert_eq!(ring.len(), vec.len());
    println!("{ring:?}");
    println!("{vec:?}");
    assert_same_elem(&ring, &vec);
}

#[test]
fn average() {
    let (ring, vec) = create_new_buffer(8);
    assert_eq!(ring.accumulator.sum, vec.iter().sum::<u32>() as u128);
    assert_eq!(ring.mean(), get_mean(&vec));
}

#[test]
fn average_wrap_arround_cap_is_windows() {
    let mut avg: DefaultRing =
        RingStatistics::try_new(4, LossyAccumulator::default()).expect("4 is a valid windows");
    let mut vec: Vec<u32> = Vec::new();

    for v in [10, 12, 9, 11, 15, 54, 23, 1, 89, 2] {
        avg.push(v);
        if vec.len() == 4 {
            vec.remove(0);
        }
        vec.push(v);

        assert_eq!(avg.mean(), get_mean(&vec), "after pushing {v}!");
    }
}

#[test]
fn average_wrap_arround_cap_is_not_windows() {
    let mut avg: DefaultRing =
        RingStatistics::try_new(6, LossyAccumulator::default()).expect("6 is a valid windows");
    let mut vec: Vec<u32> = Vec::new();

    for v in [10, 12, 9, 11, 15, 54, 23, 1, 89, 2] {
        avg.push(v);
        if vec.len() == 6 {
            vec.remove(0);
        }
        vec.push(v);

        assert_eq!(avg.mean(), get_mean(&vec), "after pushing {v}!");
    }
}

fn assert_variance(ring: &DefaultRing, vec: &[u32]) {
    let actual = ring.variance();
    let expected = get_variance(vec);

    assert!(
        (actual - expected).abs() < 1e-8,
        "variance mismatch: actual={}, expected={}",
        actual,
        expected
    );
}

#[test]
fn variance() {
    let (ring, vec) = create_new_buffer(8);
    assert_eq!(
        ring.accumulator.sum_of_squares,
        vec.iter().map(|v| v * v).sum::<u32>() as u128
    );

    assert_variance(&ring, &vec)
}

#[test]
fn variance_wrap_arround_cap_is_windows() {
    let mut ring: DefaultRing =
        RingStatistics::try_new(4, LossyAccumulator::default()).expect("4 is a valid windows");
    let mut vec: Vec<u32> = Vec::new();

    for v in [10, 12, 9, 11, 15, 54, 23, 1, 89, 2] {
        ring.push(v);
        if vec.len() == 4 {
            vec.remove(0);
        }
        vec.push(v);

        assert_variance(&ring, &vec)
    }
}

#[test]
fn variance_wrap_arround_cap_is_not_windows() {
    let mut ring: DefaultRing =
        RingStatistics::try_new(6, LossyAccumulator::default()).expect("6 is a valid windows");
    let mut vec: Vec<u32> = Vec::new();

    for v in [10, 12, 9, 11, 15, 54, 23, 1, 89, 2] {
        ring.push(v);
        if vec.len() == 6 {
            vec.remove(0);
        }
        vec.push(v);

        assert_variance(&ring, &vec)
    }
}

#[test]
fn average_and_variance_match_a_sliding_window_reference() {
    const WINDOWS: usize = 4;
    let mut ring: DefaultRing = RingStatistics::try_new(WINDOWS, LossyAccumulator::default())
        .expect("WINDOWS is a valid windows");
    let mut vec: Vec<u32> = Vec::with_capacity(WINDOWS);

    for v in [10, 12, 9, 15, 20, 8, 30, 25, 11, 4, 17, 22] {
        ring.push(v);

        vec.push(v);
        if vec.len() > WINDOWS {
            vec.remove(0);
        }

        assert_eq!(ring.mean(), get_mean(&vec), "average after pushing {v}");
        assert_variance(&ring, &vec);
    }
}

// ------------------------------- CLAUDE TEST  ------------------------------- //
const SAMPLE_VALUES: [u32; 10] = [10, 12, 9, 11, 15, 54, 23, 1, 89, 2];
const EPSILON: f64 = 1e-6;

fn get_sample_variance(data: &[u32]) -> f64 {
    let mean = get_mean(data);
    let sum_sq_diff: f64 = data
        .iter()
        .map(|&v| {
            let d = mean - v as f64;
            d * d
        })
        .sum();
    sum_sq_diff / (data.len() as f64 - 1.0)
}

fn assert_close(actual: f64, expected: f64, context: &str) {
    assert!(
        (actual - expected).abs() < EPSILON,
        "{context}: actual={actual}, expected={expected}"
    );
}

fn assert_mean_and_variance_track_sliding_window<A: Accumulator>(
    mut ring: RingStatistics<u32, A>,
    window_size: usize,
) where
    u32: Into<A::Input>,
{
    let mut reference: Vec<u32> = Vec::new();
    for v in SAMPLE_VALUES {
        ring.push(v);
        reference.push(v);
        if reference.len() > window_size {
            reference.remove(0);
        }
        assert_close(
            ring.mean(),
            get_mean(&reference),
            &format!("mean after pushing {v}"),
        );
        assert_close(
            ring.variance(),
            get_variance(&reference),
            &format!("variance after pushing {v}"),
        );
    }
}

mod construction {
    use super::*;

    #[test]
    fn new_rejects_zero_window_size() {
        assert!(RingStatistics::<i64, _>::try_new(0, LossyAccumulator::<i64>::default()).is_err());
    }
}

mod rolling_window_newest_at_front {
    use super::*;

    #[test]
    fn lossy_cap_equals_window_size() {
        let ring =
            RingStatistics::try_new(4, LossyAccumulator::<u64>::default()).expect("4 is valid");
        assert_mean_and_variance_track_sliding_window(ring, 4);
    }

    #[test]
    fn lossy_cap_exceeds_window_size() {
        let ring =
            RingStatistics::try_new(6, LossyAccumulator::<u64>::default()).expect("6 is valid");
        assert_mean_and_variance_track_sliding_window(ring, 6);
    }

    #[test]
    fn welford_cap_equals_window_size() {
        let ring = RingStatistics::try_new(4, WelfordAccumulator::default()).expect("4 is valid");
        assert_mean_and_variance_track_sliding_window(ring, 4);
    }

    #[test]
    fn welford_cap_exceeds_window_size() {
        let ring = RingStatistics::try_new(6, WelfordAccumulator::default()).expect("6 is valid");
        assert_mean_and_variance_track_sliding_window(ring, 6);
    }
}

mod empty_and_sample_variance {
    use super::*;

    #[test]
    fn stats_are_zero_before_any_push() {
        let lossy = RingStatistics::<i64, _>::try_new(4, LossyAccumulator::<i64>::default())
            .expect("4 is valid");
        assert_eq!(lossy.mean(), 0.0);
        assert_eq!(lossy.variance(), 0.0);
        assert_eq!(lossy.sample_variance(), 0.0);

        let welford = RingStatistics::<i32, _>::try_new(4, WelfordAccumulator::default())
            .expect("4 is valid");
        assert_eq!(welford.mean(), 0.0);
        assert_eq!(welford.variance(), 0.0);
        assert_eq!(welford.sample_variance(), 0.0);
    }

    #[test]
    fn sample_variance_does_not_panic_with_exactly_one_sample_lossy() {
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i64>::default()).expect("4 is valid");
        ring.push(10);
        assert_eq!(ring.sample_variance(), 0.0); // regression: used to panic (div by zero)
    }

    #[test]
    fn sample_variance_does_not_panic_with_exactly_one_sample_welford() {
        let mut ring =
            RingStatistics::try_new(4, WelfordAccumulator::default()).expect("4 is valid");
        ring.push(10);
        assert_eq!(ring.sample_variance(), 0.0); // regression: used to panic (div by zero)
    }

    #[test]
    fn sample_variance_matches_reference_after_fill() {
        let mut ring =
            RingStatistics::try_new(8, LossyAccumulator::<u64>::default()).expect("8 is valid");
        let data: Vec<u32> = (0..8).map(|i| SAMPLE_VALUES[i]).collect();
        for &v in &data {
            ring.push(v);
        }
        assert_close(
            ring.sample_variance(),
            get_sample_variance(&data),
            "sample_variance",
        );
    }
}

mod numerical_regressions {
    use super::*;

    #[test]
    fn lossy_sum_of_squares_is_not_duplicated_sum() {
        // Regression: add_sample used to do `sum_of_squares += value`
        // instead of `+= value * value`, making sum_of_squares track sum
        // a second time. True variance of [1,2,3] is 2/3.
        let mut ring =
            RingStatistics::try_new(3, LossyAccumulator::<i64>::default()).expect("3 is valid");
        for v in [1, 2, 3] {
            ring.push(v);
        }
        assert_close(ring.variance(), 2.0 / 3.0, "variance of [1,2,3]");
    }

    #[test]
    fn lossy_saturates_without_panicking_near_max() {
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i64>::default()).expect("4 is valid");
        let near_max = i64::MAX / 2;
        for _ in 0..4 {
            ring.push(near_max); // sum overflows i64::MAX -- must saturate, not panic
        }
        assert!(ring.mean() > 0.0);
    }

    #[test]
    fn welford_variance_matches_hand_computed_reference() {
        // [2,4,4,4,5,5,7,9]: mean=5, population variance=4 exactly (all
        // intermediate values here are exact powers of 1/8, so f64 holds
        // this exactly -- no epsilon needed).
        let mut ring =
            RingStatistics::try_new(8, WelfordAccumulator::default()).expect("8 is valid");
        for v in [2, 4, 4, 4, 5, 5, 7, 9] {
            ring.push(v);
        }
        assert_eq!(ring.mean(), 5.0);
        assert_eq!(ring.variance(), 4.0);

        // Push a 9th value (3): evicts the oldest (2). New window
        // [4,4,4,5,5,7,9,3], mean=5.125, variance=3.359375 (hand-computed).
        // This specifically exercises remove_sample's decremental formula.
        ring.push(3);
        assert_eq!(ring.mean(), 5.125);
        assert_eq!(ring.variance(), 3.359375);
    }
}

mod z_score {
    use super::*;

    #[test]
    fn matches_manual_computation() {
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i64>::default()).expect("4 is valid");
        for v in [10, 20, 30, 40] {
            ring.push(v);
        }
        let spashot = ring.statistics();
        let expected = (25.0 - spashot.mean) / spashot.variance.sqrt();
        assert_close(spashot.z_score(25.), expected, "z_score");
    }

    #[test]
    fn is_zero_when_std_dev_is_zero() {
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i64>::default()).expect("4 is valid");
        for _ in 0..4 {
            ring.push(50);
        }
        assert_eq!(ring.statistics().z_score(999.), 0.0);
    }
}

mod api_guarantees {
    use super::*;

    #[test]
    fn get_is_bounded_by_len_not_by_underlying_buffer_capacity() {
        let mut ring =
            RingStatistics::try_new(6, LossyAccumulator::<u64>::default()).expect("6 is valid");
        for v in 0..10 {
            ring.push(v as u32);
        }

        println!("{}", ring);
        assert_eq!(ring.len(), 6);
        assert_eq!(ring.get(6), None);
        assert!(ring.get(7).is_none());
        assert!(ring.get(5).is_some());
    }

    #[test]
    fn ring_statistics_is_send_and_sync_when_t_is() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RingStatistics<u32, LossyAccumulator<u64>>>();
        assert_send_sync::<RingStatistics<i32, WelfordAccumulator>>();
    }
}

mod stability_detection {
    use super::*;

    #[test]
    fn saturation_is_reported() {
        // (i64::MAX / 2)² overflows i64 on the very first sample.
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i64>::default()).expect("4 is valid");
        for _ in 0..4 {
            ring.push(i64::MAX / 2);
        }
        assert_eq!(ring.status_accumulator(), StatusAccumulator::Overflow);
        assert!(!ring.is_accumulator_stable());
    }

    #[test]
    fn cancellation_is_reported() {
        // Large magnitude (~2e9), tiny spread (~1) => std/mean ~5e-10.
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i128>::default()).expect("4 is valid");
        for v in [
            2_000_000_000i32,
            2_000_000_001,
            2_000_000_000,
            2_000_000_001,
        ] {
            ring.push(v);
        }
        assert_eq!(ring.status_accumulator(), StatusAccumulator::Unstable);
    }

    #[test]
    fn healthy_data_is_ok() {
        // Durations in ns: ~1 s with ~50 ms of jitter.
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<u128>::default()).expect("4 is valid");
        for v in [1_000_000_000u128, 1_050_000_000, 950_000_000, 1_000_000_000] {
            ring.push(v);
        }
        assert_eq!(ring.status_accumulator(), StatusAccumulator::Okay);
    }

    #[test]
    fn constant_window_is_ok() {
        // A variance of exactly zero is a legitimate result, not cancellation.
        let mut ring =
            RingStatistics::try_new(4, LossyAccumulator::<i64>::default()).expect("4 is valid");
        for _ in 0..8 {
            ring.push(5i32);
        }
        assert_eq!(ring.variance(), 0.0);
        assert_eq!(ring.status_accumulator(), StatusAccumulator::Okay);
        assert!(ring.is_accumulator_stable());
    }
}

mod construction_errors {
    use super::*;

    #[test]
    fn windows_of_two_or_less_are_refused() {
        for window_size in 0..=2 {
            let result =
                RingStatistics::<i32, _>::try_new(window_size, LossyAccumulator::<i64>::default());
            assert!(
                matches!(result, Err(WindowError::TooSmall { window_size: w }) if w == window_size)
            );
        }
        assert!(RingStatistics::<i32, _>::try_new(3, LossyAccumulator::<i64>::default()).is_ok());
    }

    #[test]
    fn window_error_names_the_size() {
        let err = WindowError::TooSmall { window_size: 2 };
        assert_eq!(
            err.to_string(),
            "window of 2 samples is too small, the minimum is 3"
        );
    }
}
