//! Rolling mean and variance over a fixed window, maintained in O(1) per push.
//!
//! [`RingStatistics`] keeps the window in a [`RingBuffer`] and an
//! [`Accumulator`] next to it: a push adds the new sample to the accumulator
//! and removes the evicted one. Two accumulators are provided:
//! [`LossyAccumulator`] (running sums, exact on integers, reports its own
//! health) and [`WelfordAccumulator`] (numerically stable, always `f64`).

use std::ops::Sub;

use crate::{InitError, RingBuffer};

// ---------------------------------------------------------------- scalars
/// A numeric type an accumulator can sum into.
///
/// Implemented for `i64`, `i128`, `u64`, `u128`, `f32` and `f64`. The
/// arithmetic saturates instead of overflowing and reports when it did, so an
/// accumulator can flag the result instead of panicking or wrapping.
pub trait Scalar: Copy + Sub<Output = Self> + PartialEq {
    /// The additive identity.
    const ZERO: Self;
    /// The multiplicative identity, used to count samples.
    const ONE: Self;
    /// `*self += rhs`, saturating. Returns `true` if the result saturated.
    fn assign_saturating_add(&mut self, rhs: Self) -> bool;
    /// `*self -= rhs`, saturating. Returns `true` if the result saturated.
    fn assign_saturating_sub(&mut self, rhs: Self) -> bool;
    /// `self * rhs`, saturating, along with `true` if the result saturated.
    fn saturating_mul_flagged(self, rhs: Self) -> (Self, bool);
    /// `numerator / denominator` as an `f64`, losing as little precision as
    /// the type allows. For integers the quotient and the remainder are
    /// converted separately, so a sum past 2^53 does not lose its low bits
    /// before the division.
    fn div_to_f64(numerator: Self, denominator: Self) -> f64;
}

macro_rules! impl_scalar_integer {
    ($($t:ty),+ $(,)?) => {$(
        impl Scalar for $t {
            const ZERO: Self = 0;
            const ONE: Self = 1;

            fn assign_saturating_add(&mut self, rhs: Self) -> bool {
                match self.checked_add(rhs) {
                    Some(value) => {
                        *self = value;
                        false
                    },
                    None => {
                        *self = self.saturating_add(rhs);
                        true
                    }
                }
            }

            fn assign_saturating_sub(&mut self, rhs: Self) -> bool {
                match self.checked_sub(rhs) {
                    Some(value) => {
                        *self = value;
                        false
                    },
                    None => {
                        *self = self.saturating_sub(rhs);
                        true
                    }
                }
            }

            fn saturating_mul_flagged(self, rhs: Self) -> (Self, bool) {
                match self.checked_mul(rhs) {
                    Some(value) => (value, false),
                    None => (<$t>::saturating_mul(self, rhs), true)
                }
            }

            fn div_to_f64(numerator: Self, denominator: Self) -> f64 {
                let q = numerator / denominator;
                let r = numerator % denominator;

                q as f64 + r as f64 / denominator as f64
            }
        }
    )+};
}

macro_rules! impl_scalar_float {
    ($($t:ty),+ $(,)?) => {$(
        impl Scalar for $t {
            const ZERO: Self = 0.0;
            const ONE: Self = 1.0;

            fn assign_saturating_add(&mut self, rhs: Self) -> bool {
                let raw = *self + rhs;
                let clamped = !(<$t>::MIN..=<$t>::MAX).contains(&raw);
                *self = raw.clamp(<$t>::MIN, <$t>::MAX);
                clamped
            }

            fn assign_saturating_sub(&mut self, rhs: Self) -> bool {
                let raw = *self - rhs;
                let clamped = !(<$t>::MIN..=<$t>::MAX).contains(&raw);
                *self = raw.clamp(<$t>::MIN, <$t>::MAX);
                clamped
            }

            fn saturating_mul_flagged(self, rhs: Self) -> (Self, bool) {
                let raw = self * rhs;
                let clamped = !(<$t>::MIN..=<$t>::MAX).contains(&raw);
                (raw.clamp(<$t>::MIN, <$t>::MAX), clamped)
            }

            fn div_to_f64(numerator: Self, denominator: Self) -> f64 {
                numerator as f64 / denominator as f64
            }
        }
    )+};
}

impl_scalar_integer!(i64, i128, u64, u128);
impl_scalar_float!(f32, f64);

// ----------------------------------------------------------- accumulators
/// Health of an accumulator's results.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusAccumulator {
    /// The results can be trusted.
    Okay,
    /// A saturating operation clamped: the sums, and everything derived from
    /// them, are wrong. [`Accumulator::reset`] clears it.
    Overflow,
    /// `E[X²] - E[X]²` has lost most of its significant digits to
    /// cancellation (large magnitude, tiny relative spread). The variance is
    /// unreliable and may even come out negative.
    Unstable,
}

/// Incremental statistics over the samples currently in a window.
pub trait Accumulator {
    /// The type the accumulator sums into.
    type Input: Scalar;

    /// Adds a sample entering the window.
    fn add_sample(&mut self, value: Self::Input);
    /// Removes a sample leaving the window.
    ///
    /// Must only be called for a value previously passed to `add_sample` and
    /// not yet removed: implementations may assume at least one sample is
    /// currently tracked.
    fn remove_sample(&mut self, value: Self::Input);
    /// Sum of the tracked samples.
    fn sum(&self) -> Self::Input;
    /// Mean of the tracked samples, `0.0` when there are none.
    fn mean(&self) -> f64;
    /// Population variance of the tracked samples, `0.0` when there are none.
    fn variance(&self) -> f64;
    /// Sample (Bessel-corrected) variance, `0.0` with fewer than two samples.
    fn sample_variance(&self) -> f64;
    /// Population standard deviation.
    fn std(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Whether the results can be trusted.
    fn status(&self) -> StatusAccumulator;
    /// Clears every tracked sample.
    ///
    /// Incremental accumulators drift the longer they run; clearing and
    /// re-adding the window brings the drift back to zero.
    fn reset(&mut self);
}

/// Running sum and sum of squares: fast, exact on integers, lossy on floats.
///
/// Saturates instead of overflowing, so accumulation alone can never panic,
/// and reports it through [`StatusAccumulator::Overflow`]. Returns `0.0` for
/// `mean`/`variance`/`sample_variance` when there is not enough data yet,
/// rather than `None`: treat `0.0` as "no data" at the call site if that
/// distinction matters to you.
#[derive(Debug, Clone, Copy)]
pub struct LossyAccumulator<U: Scalar> {
    sum: U,
    sum_of_squares: U,
    fill_quantity: U,

    overflowed: bool,
}
impl<U: Scalar> Default for LossyAccumulator<U> {
    fn default() -> Self {
        Self {
            sum: U::ZERO,
            sum_of_squares: U::ZERO,
            fill_quantity: U::ZERO,

            overflowed: false,
        }
    }
}

/// Above this `mean² / variance` ratio, `E[X²] - E[X]²` has cancelled away
/// too many of the 53 bits of an `f64` significand to be trusted.
const CANCELLATION_RATIO_LIMIT: f64 = 4.5e11;

impl<U: Scalar> Accumulator for LossyAccumulator<U> {
    type Input = U;

    fn reset(&mut self) {
        *self = Self::default()
    }

    fn add_sample(&mut self, value: U) {
        self.overflowed |= self.sum.assign_saturating_add(value);
        let (square, mul_saturated) = value.saturating_mul_flagged(value);
        self.overflowed |= mul_saturated;
        self.overflowed |= self.sum_of_squares.assign_saturating_add(square);
        self.fill_quantity.assign_saturating_add(U::ONE);
    }

    fn remove_sample(&mut self, value: U) {
        if self.fill_quantity == U::ONE {
            // The window is now empty: start from a clean state rather than
            // subtracting, which also clears a past overflow.
            *self = Self::default();
            return;
        }
        self.overflowed |= self.sum.assign_saturating_sub(value);
        let (square, mul_saturated) = value.saturating_mul_flagged(value);
        self.overflowed |= mul_saturated;
        self.overflowed |= self.sum_of_squares.assign_saturating_sub(square);
        self.fill_quantity.assign_saturating_sub(U::ONE);
    }

    fn sum(&self) -> Self::Input {
        self.sum
    }

    fn mean(&self) -> f64 {
        if self.fill_quantity == U::ZERO {
            return 0.;
        }
        U::div_to_f64(self.sum, self.fill_quantity)
    }

    fn variance(&self) -> f64 {
        if self.fill_quantity == U::ZERO {
            return 0.;
        }
        let mean = U::div_to_f64(self.sum, self.fill_quantity);
        let var = U::div_to_f64(self.sum_of_squares, self.fill_quantity) - mean * mean;
        var.max(0.)
    }

    fn sample_variance(&self) -> f64 {
        if self.fill_quantity == U::ZERO || self.fill_quantity == U::ONE {
            return 0.0;
        }

        let var = self.variance() * U::div_to_f64(self.fill_quantity, self.fill_quantity - U::ONE);
        var.max(0.)
    }

    fn status(&self) -> StatusAccumulator {
        if self.overflowed {
            return StatusAccumulator::Overflow;
        }

        if self.fill_quantity == U::ZERO {
            return StatusAccumulator::Okay;
        }

        let mean = U::div_to_f64(self.sum, self.fill_quantity);
        let e_x2 = U::div_to_f64(self.sum_of_squares, self.fill_quantity);
        let variance = e_x2 - mean * mean;

        if variance <= 0.0 {
            // Either a constant window, a legitimate zero, or cancellation that
            // wiped out a small spread. The accumulator's own arithmetic tells
            // them apart: a constant window has `n·Σx² == (Σx)²` exactly.
            let (n_sum_sq, overflow_a) = self
                .fill_quantity
                .saturating_mul_flagged(self.sum_of_squares);
            let (sum_sq, overflow_b) = self.sum.saturating_mul_flagged(self.sum);
            if !overflow_a && !overflow_b && n_sum_sq == sum_sq {
                return StatusAccumulator::Okay;
            }
            return StatusAccumulator::Unstable;
        }
        if (mean * mean) / variance > CANCELLATION_RATIO_LIMIT {
            return StatusAccumulator::Unstable;
        }
        StatusAccumulator::Okay
    }
}

/// Welford's online algorithm: better numerical stability than
/// [`LossyAccumulator`] (no `E[X²] - E[X]²` cancellation), at the cost of
/// always operating in `f64` internally regardless of the input type.
///
/// `RingStatistics<T, WelfordAccumulator>` therefore needs `T: Into<f64>`:
/// small integer types and `f32` qualify; `i64`, `u64`, `i128` and `u128` do
/// not, since std has no lossless conversion from those into `f64`.
#[derive(Debug, Clone, Copy, Default)]
pub struct WelfordAccumulator {
    mean: f64,
    m2: f64,
    count: f64,
}

impl Accumulator for WelfordAccumulator {
    type Input = f64;

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn add_sample(&mut self, value: Self::Input) {
        self.count += 1.;
        let old_mean = self.mean;

        self.mean += (value - self.mean) / self.count;
        self.m2 += (value - old_mean) * (value - self.mean);
    }

    fn remove_sample(&mut self, value: Self::Input) {
        self.count -= 1.;
        let old_mean = self.mean;

        if self.count == 0. {
            self.mean = 0.;
            self.m2 = 0.;
        } else {
            self.mean -= (value - self.mean) / self.count;
            self.m2 -= (value - old_mean) * (value - self.mean);
        }
    }

    fn sum(&self) -> f64 {
        self.mean * self.count
    }

    fn mean(&self) -> f64 {
        if self.count == 0. {
            return 0.;
        }
        self.mean
    }

    fn variance(&self) -> f64 {
        if self.count == 0. {
            return 0.;
        }
        self.m2 / self.count
    }

    fn sample_variance(&self) -> f64 {
        if self.count <= 1. {
            return 0.;
        }
        self.m2 / (self.count - 1.)
    }

    fn status(&self) -> StatusAccumulator {
        StatusAccumulator::Okay
    }
}

// --------------------------------------------------------- rolling window
/// A fixed-size rolling window whose statistics are maintained incrementally.
///
/// Samples of type `T` go in; the accumulator works in `A::Input`, which is
/// deliberately a *wider* type. Two independent limits force that split.
///
/// # Why the accumulator is wider than the sample
///
/// [`LossyAccumulator`] keeps a running `sum` and `sum_of_squares`. Over a
/// window of `n` samples bounded by `m` they grow as `n·m` and `n·m²`, so it is
/// the squares that overflow first: with `u32` samples a single value above
/// 65 535 already puts `sum_of_squares` past `u32::MAX`. The accumulator must
/// therefore be at least twice the sample's width:
///
/// | Samples (`T`) | Accumulator (`A::Input`) |
/// |---------------|--------------------------|
/// | `i8`…`i32`    | `i64` or `i128`          |
/// | `u8`…`u32`    | `u64` or `u128`          |
/// | `f32`         | `f64`                    |
///
/// Overflow is not a panic: [`LossyAccumulator`] saturates and reports it
/// through [`StatusAccumulator::Overflow`]. The numbers stop being meaningful
/// before they stop being produced, which is exactly why the status has to be
/// read alongside them.
///
/// # Where `f64` stops being exact
///
/// `mean` and `variance` are ratios, so they come back as `f64`. An `f64` has a
/// 53-bit significand (1 sign bit, 11 exponent, 52 stored mantissa plus one
/// implicit), so every integer up to 2^53 is represented exactly and beyond
/// that the gap between neighbouring values grows past 1.
///
/// A `u64` or `u128` sum can therefore exceed what an `f64` can name. Widening
/// the sum and dividing afterwards would discard those bits, so
/// [`Scalar::div_to_f64`] divides first: it splits the ratio into quotient and
/// remainder, converts each on its own, and only the remainder, always smaller
/// than the divisor, goes through a lossy conversion.
///
/// # Ordering
///
/// [`Self::push`] inserts at the head: index 0 is the **newest** sample and
/// [`Self::last`] the oldest. Eviction drops the oldest and removes it from the
/// accumulator in the same step.
pub struct RingStatistics<T, A: Accumulator> {
    buffer: RingBuffer<T>,
    window_size: usize,
    accumulator: A,
}

/// Why a [`RingStatistics`] could not be created.
#[derive(Debug)]
pub enum WindowError {
    /// The window must hold at least three samples.
    TooSmall {
        /// The size that was asked for.
        window_size: usize,
    },
    /// The underlying ring buffer could not be allocated.
    Capacity(InitError),
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { window_size } => {
                write!(
                    f,
                    "window of {window_size} samples is too small, the minimum is 3"
                )
            }
            Self::Capacity(err) => write!(f, "cannot allocate the window: {err}"),
        }
    }
}

impl std::error::Error for WindowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TooSmall { .. } => None,
            Self::Capacity(err) => Some(err),
        }
    }
}

impl<T, A: Accumulator> RingStatistics<T, A>
where
    T: Copy + Into<A::Input>,
{
    /// Creates an empty window of `window_size` samples (at least 3).
    pub fn try_new(window_size: usize, accumulator: A) -> Result<Self, WindowError> {
        if window_size <= 2 {
            return Err(WindowError::TooSmall { window_size });
        }

        Ok(RingStatistics {
            buffer: RingBuffer::try_new(window_size).map_err(WindowError::Capacity)?,
            window_size,

            accumulator,
        })
    }

    /// Clears the accumulator and feeds it the current window again.
    fn init(&mut self) {
        self.accumulator.reset();
        let init_len = self.window_size.min(self.buffer.len());
        self.buffer
            .iter_head_to_tail()
            .rev()
            .take(init_len)
            .for_each(|v| self.accumulator.add_sample((*v).into()));
    }

    /// Adds a sample, evicting the oldest one once the window is full.
    pub fn push(&mut self, value: T) {
        if let Some(evicted) = self.buffer.push_head_evicting(value) {
            self.accumulator.remove_sample(evicted.into());
        }
        self.accumulator.add_sample(value.into());
    }

    /// Every statistic at once, read from the same state.
    pub fn statistics(&self) -> StatisticsSnapshot<A::Input> {
        StatisticsSnapshot {
            count: self.len(),
            sum: self.sum(),
            mean: self.mean(),
            variance: self.variance(),
            status: self.status_accumulator(),
        }
    }

    /// Sum of the samples in the window.
    pub fn sum(&self) -> A::Input {
        self.accumulator.sum()
    }
    /// Mean of the samples in the window, `0.0` when it is empty.
    pub fn mean(&self) -> f64 {
        self.accumulator.mean()
    }
    /// Population variance of the window.
    pub fn variance(&self) -> f64 {
        self.accumulator.variance()
    }
    /// Sample (Bessel-corrected) variance of the window.
    pub fn sample_variance(&self) -> f64 {
        self.accumulator.sample_variance()
    }
    /// Population standard deviation of the window.
    pub fn std(&self) -> f64 {
        self.accumulator.std()
    }

    /// Rebuilds the accumulator from the samples currently in the window,
    /// clearing any drift or overflow it accumulated.
    pub fn reset_accumulator(&mut self) {
        self.init();
    }
    /// Health of the accumulator's results.
    pub fn status_accumulator(&self) -> StatusAccumulator {
        self.accumulator.status()
    }
    /// True when the results can be trusted.
    pub fn is_accumulator_stable(&self) -> bool {
        matches!(self.status_accumulator(), StatusAccumulator::Okay)
    }
    /// Replaces the accumulator and feeds it the current window.
    pub fn new_accumulator(&mut self, accumulator: A) {
        self.accumulator = accumulator;
        self.init();
    }

    /// The `i`th newest sample, by value.
    pub fn get(&self, i: usize) -> Option<T> {
        self.buffer.get(i)
    }
    /// The `i`th newest sample, by reference.
    pub fn peek(&self, i: usize) -> Option<&T> {
        self.buffer.peek(i)
    }
    /// The newest sample.
    pub fn first(&self) -> Option<T> {
        self.buffer.get(0)
    }
    /// The oldest sample.
    pub fn last(&self) -> Option<T> {
        self.buffer.get(self.len().checked_sub(1)?)
    }
    /// Iterates the window from the newest sample to the oldest.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.buffer.iter_head_to_tail()
    }

    /// Samples currently in the window.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }
    /// True before the first push.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
    /// True once the window holds `window_size` samples.
    pub fn is_full(&self) -> bool {
        self.buffer.is_full()
    }
    /// Pushes left before the window is full.
    pub fn samples_until_full(&self) -> usize {
        self.window_size.saturating_sub(self.buffer.len())
    }
    /// The window size asked for at construction.
    pub fn window_size(&self) -> usize {
        self.window_size
    }
}
impl<T: Clone, A: Accumulator + Clone> Clone for RingStatistics<T, A> {
    fn clone(&self) -> Self {
        RingStatistics {
            buffer: self.buffer.clone(),
            window_size: self.window_size,
            accumulator: self.accumulator.clone(),
        }
    }
}

/// Statistics of a window at one point in time, see
/// [`RingStatistics::statistics`].
#[derive(Debug, Clone, Copy)]
pub struct StatisticsSnapshot<U> {
    /// Samples in the window.
    pub count: usize,
    /// Their sum.
    pub sum: U,
    /// Their mean.
    pub mean: f64,
    /// Their population variance.
    pub variance: f64,
    /// Whether the figures above can be trusted.
    pub status: StatusAccumulator,
}
impl<U> StatisticsSnapshot<U> {
    /// How many standard deviations `value` sits from the mean. `0.0` when the
    /// window has no spread.
    pub fn z_score(&self, value: f64) -> f64 {
        let std_dev = self.variance.sqrt();
        if std_dev == 0. {
            return 0.;
        }
        (value - self.mean) / std_dev
    }

    /// Standard deviation relative to the mean. `0.0` when the mean is zero,
    /// where the ratio is undefined.
    pub fn coef_variation(&self) -> f64 {
        if self.mean == 0. {
            return 0.;
        }
        self.variance.sqrt() / self.mean
    }
}

// -------------------------------------------------------- debug / display
/// Made by IA. too lazy to do it by hands!
const WINDOW_PREVIEW_EDGE_LEN: usize = 3;

struct WindowPreview<'a, T, A: Accumulator> {
    ring: &'a RingStatistics<T, A>,
    edge_len: usize,
}

impl<T, A> std::fmt::Debug for WindowPreview<'_, T, A>
where
    T: std::fmt::Debug + Copy + Into<A::Input>,
    A: Accumulator,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.ring.len();
        let mut list = f.debug_list();

        let head_end = len.min(self.edge_len);
        for i in 0..head_end {
            if let Some(value) = self.ring.peek(i) {
                list.entry(value);
            }
        }

        if len > self.edge_len * 2 {
            list.entry(&format_args!("..."));
            for i in (len - self.edge_len)..len {
                if let Some(value) = self.ring.peek(i) {
                    list.entry(value);
                }
            }
        } else if len > head_end {
            for i in head_end..len {
                if let Some(value) = self.ring.peek(i) {
                    list.entry(value);
                }
            }
        }

        list.finish()
    }
}

impl<T, A> std::fmt::Debug for RingStatistics<T, A>
where
    T: std::fmt::Debug + Copy + Into<A::Input>,
    A: Accumulator + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingStatistics")
            .field("window_size", &self.window_size)
            .field("buffer_len", &self.buffer.len())
            .field("buffer_capacity", &self.buffer.asked_capacity())
            .field("effective_len", &self.len())
            .field("samples_until_full", &self.samples_until_full())
            .field("status", &self.status_accumulator())
            .field("accumulator", &self.accumulator)
            .field(
                "window",
                &WindowPreview {
                    ring: self,
                    edge_len: WINDOW_PREVIEW_EDGE_LEN,
                },
            )
            .finish()
    }
}

impl<T, A> std::fmt::Display for RingStatistics<T, A>
where
    T: std::fmt::Debug + Copy + Into<A::Input>,
    A: Accumulator,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let preview = WindowPreview {
            ring: self,
            edge_len: WINDOW_PREVIEW_EDGE_LEN,
        };

        if f.alternate() {
            writeln!(f, "RingStatistics")?;
            writeln!(
                f,
                "  size               : {}/{} (buffer {}/{})",
                self.len(),
                self.window_size,
                self.buffer.len(),
                self.buffer.asked_capacity()
            )?;
            writeln!(f, "  samples_until_full : {}", self.samples_until_full())?;
            writeln!(f, "  status             : {:?}", self.status_accumulator())?;
            writeln!(
                f,
                "  mean / std         : {:.6} / {:.6}",
                self.mean(),
                self.std()
            )?;
            writeln!(
                f,
                "  variance / sample  : {:.6} / {:.6}",
                self.variance(),
                self.sample_variance()
            )?;
            write!(f, "  values (newest->oldest): {preview:?}")
        } else {
            write!(
                f,
                "RingStatistics {{ size: {}/{}, status: {:?}, mean: {:.6}, std: {:.6}, values: {:?} }}",
                self.len(),
                self.window_size,
                self.status_accumulator(),
                self.mean(),
                self.std(),
                preview
            )
        }
    }
}

#[cfg(test)]
#[path = "tests/ring_statistics.rs"]
mod ring_statistics;
