//! File done with the help of IA.
//!
//! Latency distribution for the one-way path: median, p99, p99.9.
//!
//!   cargo bench -p channel --bench latency
//!   cargo bench -p channel --bench latency --features "crossbeam-queue crossbeam-channel"
//!
//! This is not a `#[bench]`. libtest reports a mean and a standard deviation
//! over aggregated iterations and discards the distribution, which is the only
//! thing a tail lives in. `harness = false`, own `main`, stable toolchain.
//!
//! Scope: this measures the **tail**. Reading the clock costs more than the
//! operation being timed, so the median here is dominated by the instrument and
//! only differences between links on the same row mean anything. For
//! per-operation cost use the `#[bench]` targets instead.
//!
//! Every measured pass also prints a `#DATA,` line with the raw figures, so a
//! downstream script never has to parse the formatted table.

#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(not(target_arch = "x86_64"))]
compile_error!("this harness reads the TSC directly and is x86_64 only");

use std::{
    hint::{black_box, spin_loop},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::sync_channel,
    },
    time::{Duration, Instant},
};

use wincore::thread::{
    Builder,
    affinity::ProcessorId,
    priority::{BoostPolicy, ThreadPriority},
};

// ---------------------------------------------------------------- knobs

const CAPACITY: usize = 1024;
const WARMUP: usize = 200_000;
const SAMPLES: usize = 4_000_000;

/// Measured passes per configuration. The first pass is always discarded: it
/// carries the core ramp-up and whatever the allocator and the OS were doing.
const PASSES: usize = 11;

const CONSUMER_CPU: u8 = 4;
const CPU_GROUP: u16 = 0;

/// One physical core per producer. On the usual enumeration the even logical
/// CPUs are the distinct physical cores; `check_topology` complains if this list
/// contradicts that.
const CORES_EXCLUSIVE: [u8; 3] = [2, 6, 0];

/// Both hyper-threads of those same three cores. Producers then contend for
/// execution units as well as for the ring, and one can be preempted in the
/// middle of a write. This is the production shape: more threads than cores.
const CORES_OVERSUBSCRIBED: [u8; 6] = [2, 3, 6, 7, 0, 1];

/// Control experiment: two producers on the two hyper-threads of one core,
/// against two producers on two separate cores, at the same offered load. The
/// only variable is whether the pair shares an L1 and a pipeline.
const CORES_SIBLING_PAIR: [u8; 2] = [2, 3];
const CORES_DISTINCT_PAIR: [u8; 2] = [2, 6];

const FAN_IN: [usize; 3] = [1, 2, 3];
const RATES_HZ: [u64; 7] = [
    500_000, 1_000_000, 2_000_000, 4_000_000, 6_000_000, 8_000_000, 10_000_000,
];

/// Total offered load for every multi-producer table, split across the
/// producers, so the only variable between rows is how many threads deliver it.
const FAN_IN_RATE: u64 = 4_000_000;

const CALIBRATION_WINDOW: Duration = Duration::from_millis(500);
const OVERHEAD_PROBES: usize = 100_000;

/// Enough for every producer thread to be spinning before the schedule starts,
/// so a slow spawn does not surface as latency on the first messages.
const STARTUP_GRACE: Duration = Duration::from_millis(200);

const WANTED_WORKING_SET_MIN: usize = 128 * 1024 * 1024;
const WANTED_WORKING_SET_MAX: usize = 512 * 1024 * 1024;

/// A pass that kept up is flat. One that did not shows the backlog growing from
/// the first sample to the last, by more than this factor.
const STEADY_STATE_RATIO: u64 = 4;

/// Sanity band for the measured TSC frequency. Outside it, something is wrong
/// with the clock and every number below is meaningless.
const PLAUSIBLE_CYCLES_PER_NS: std::ops::RangeInclusive<f64> = 0.1..=20.0;

// ---------------------------------------------------------------- clock

mod clock {
    use super::*;

    /// Reads the invariant TSC.
    ///
    /// `rdtscp` waits for earlier instructions to retire, so it needs no
    /// separate `lfence` and costs about a third less than the pair.
    #[inline(always)]
    pub(crate) fn now() -> u64 {
        let mut aux = 0u32;
        // SAFETY: rdtscp exists on every x86_64 part since Nehalem and K10, and
        // writes only through `aux`, a live local.
        unsafe { core::arch::x86_64::__rdtscp(&mut aux) }
    }

    /// TSC cycles per nanosecond, measured against the OS clock once at startup.
    pub(crate) fn calibrate() -> f64 {
        let wall = Instant::now();
        let start = now();
        std::thread::sleep(CALIBRATION_WINDOW);
        let cycles = now().wrapping_sub(start);
        let elapsed = wall.elapsed().as_nanos();
        if elapsed == 0 {
            return 0.0;
        }
        cycles as f64 / elapsed as f64
    }

    /// Median cost of one `now()` call, reported so the reader can subtract it.
    pub(crate) fn overhead_cycles() -> u64 {
        let mut probes = vec![0u64; OVERHEAD_PROBES];
        for probe in probes.iter_mut() {
            let a = now();
            let b = now();
            *probe = b.saturating_sub(a);
        }
        probes.sort_unstable();
        probes[OVERHEAD_PROBES / 2]
    }

    pub(crate) fn cycles(duration: Duration, cycles_per_ns: f64) -> u64 {
        (duration.as_nanos() as f64 * cycles_per_ns) as u64
    }

    /// Cross-core sanity: every cross-thread figure in this harness is a
    /// difference between a TSC read on the producer core and one on the
    /// consumer core. That is only meaningful if the two share a clock domain.
    ///
    /// Reads on this thread, then on a thread pinned to the consumer CPU, then
    /// here again. The middle value must sit between the outer two.
    pub(crate) fn check_cross_core() {
        let (tx, rx) = sync_channel::<u64>(1);
        let spawned = Builder::new()
            .name("tsc-probe".to_string())
            .pin_to(match ProcessorId::try_new(CONSUMER_CPU as u32, CPU_GROUP) {
                Some(id) => id,
                None => {
                    eprintln!("[bench] cannot build a ProcessorId for the consumer CPU; skipping the TSC check");
                    return;
                }
            })
            .spawn(move || {
                let _ = tx.send(now());
            });

        let before = now();
        let middle = match spawned {
            Ok(spawned) => {
                let value = rx.recv().ok();
                let _ = spawned.handle.join();
                value
            }
            Err(source) => {
                eprintln!("[bench] TSC probe thread did not start ({source}); skipping the check");
                return;
            }
        };
        let after = now();

        match middle {
            Some(middle) if middle >= before && middle <= after => {}
            Some(middle) => eprintln!(
                "[bench] WARNING: the TSC is not consistent across cores \
                 (here {before}, on CPU {CONSUMER_CPU} {middle}, here again {after}). \
                 Every cross-thread figure below is meaningless."
            ),
            None => eprintln!("[bench] TSC probe returned nothing; skipping the check"),
        }
    }
}

// ---------------------------------------------------------------- stats

#[derive(Clone, Copy)]
struct Summary {
    count: usize,
    min: f64,
    p50: f64,
    p90: f64,
    p99: f64,
    p999: f64,
    max: f64,
}

impl Summary {
    fn field(&self, index: usize) -> f64 {
        match index {
            0 => self.min,
            1 => self.p50,
            2 => self.p90,
            3 => self.p99,
            4 => self.p999,
            _ => self.max,
        }
    }
}

/// Sorts in place: an exact quantile over a few million `u64` costs less than
/// any sketch and leaves nothing to argue about.
fn summarize(samples: &mut [u64], cycles_per_ns: f64) -> Option<Summary> {
    if samples.is_empty() || cycles_per_ns <= 0.0 {
        return None;
    }
    samples.sort_unstable();

    let at = |q: f64| {
        let idx = ((samples.len() - 1) as f64 * q).round() as usize;
        samples[idx.min(samples.len() - 1)] as f64 / cycles_per_ns
    };

    Some(Summary {
        count: samples.len(),
        min: samples[0] as f64 / cycles_per_ns,
        p50: at(0.50),
        p90: at(0.90),
        p99: at(0.99),
        p999: at(0.999),
        max: samples[samples.len() - 1] as f64 / cycles_per_ns,
    })
}

/// Did the link keep up with the offered load?
///
/// Samples are stored in arrival order, so a pass that fell behind has a
/// visibly heavier end than start. Must be called before `summarize` sorts them.
fn kept_up(samples: &[u64]) -> bool {
    let chunk = samples.len() / 10;
    if chunk == 0 {
        return true;
    }
    let mean = |slice: &[u64]| slice.iter().map(|&v| v as u128).sum::<u128>() / chunk as u128;
    mean(&samples[samples.len() - chunk..])
        <= mean(&samples[..chunk]).saturating_mul(STEADY_STATE_RATIO as u128)
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

// ---------------------------------------------------------------- links

#[derive(Clone, Copy)]
struct Stamped {
    /// When this message was *due*, not when it was handed over.
    due: u64,
    seq: u64,
}

/// What a non-blocking receive found.
///
/// `Busy` is the interesting one: the head slot is reserved by a producer that
/// has taken its ticket and not published yet. Only `channel::mpsc` can report
/// it; every other link folds that state into "nothing here".
enum Recv {
    Value(Stamped),
    Busy,
    Empty,
}

/// One bounded, non-blocking link, so every implementation goes through the
/// exact same producer and consumer loops.
trait Link {
    const NAME: &'static str;
    type Tx: Send + 'static;
    type Rx: Send + 'static;

    fn create() -> (Self::Tx, Self::Rx);
    fn try_send(tx: &mut Self::Tx, msg: Stamped) -> bool;
    fn try_recv(rx: &mut Self::Rx) -> Recv;

    /// `None` when the producer end cannot be duplicated. `channel::spsc`
    /// returns `None` by construction, and that is a guarantee, not a gap.
    fn clone_tx(tx: &Self::Tx) -> Option<Self::Tx>;
}

struct Spsc;
impl Link for Spsc {
    const NAME: &'static str = "channel::spsc";
    type Tx = channel::spsc::Producer<Stamped, CAPACITY>;
    type Rx = channel::spsc::Consumer<Stamped, CAPACITY>;

    fn create() -> (Self::Tx, Self::Rx) {
        channel::spsc::channel()
    }
    fn try_send(tx: &mut Self::Tx, msg: Stamped) -> bool {
        tx.push(msg).is_ok()
    }
    fn try_recv(rx: &mut Self::Rx) -> Recv {
        match rx.pop() {
            Some(v) => Recv::Value(v),
            None => Recv::Empty,
        }
    }
    fn clone_tx(_: &Self::Tx) -> Option<Self::Tx> {
        None
    }
}

struct Mpsc;
impl Link for Mpsc {
    const NAME: &'static str = "channel::mpsc";
    type Tx = channel::mpsc::Producer<Stamped, CAPACITY>;
    type Rx = channel::mpsc::Consumer<Stamped, CAPACITY>;

    fn create() -> (Self::Tx, Self::Rx) {
        channel::mpsc::channel()
    }
    fn try_send(tx: &mut Self::Tx, msg: Stamped) -> bool {
        tx.push(msg).is_ok()
    }
    fn try_recv(rx: &mut Self::Rx) -> Recv {
        use channel::mpsc::PopOutcome;
        match rx.try_pop() {
            PopOutcome::Value(v) => Recv::Value(v),
            PopOutcome::Busy => Recv::Busy,
            PopOutcome::Empty => Recv::Empty,
        }
    }
    fn clone_tx(tx: &Self::Tx) -> Option<Self::Tx> {
        Some(tx.clone())
    }
}

struct StdMpsc;
impl Link for StdMpsc {
    const NAME: &'static str = "std::sync::mpsc";
    type Tx = std::sync::mpsc::SyncSender<Stamped>;
    type Rx = std::sync::mpsc::Receiver<Stamped>;

    fn create() -> (Self::Tx, Self::Rx) {
        sync_channel(CAPACITY)
    }
    fn try_send(tx: &mut Self::Tx, msg: Stamped) -> bool {
        tx.try_send(msg).is_ok()
    }
    fn try_recv(rx: &mut Self::Rx) -> Recv {
        match rx.try_recv() {
            Ok(v) => Recv::Value(v),
            Err(_) => Recv::Empty,
        }
    }
    fn clone_tx(tx: &Self::Tx) -> Option<Self::Tx> {
        Some(tx.clone())
    }
}

#[cfg(feature = "crossbeam-channel")]
struct CrossbeamChannel;
#[cfg(feature = "crossbeam-channel")]
impl Link for CrossbeamChannel {
    const NAME: &'static str = "crossbeam-channel";
    type Tx = crossbeam_channel::Sender<Stamped>;
    type Rx = crossbeam_channel::Receiver<Stamped>;

    fn create() -> (Self::Tx, Self::Rx) {
        crossbeam_channel::bounded(CAPACITY)
    }
    fn try_send(tx: &mut Self::Tx, msg: Stamped) -> bool {
        tx.try_send(msg).is_ok()
    }
    fn try_recv(rx: &mut Self::Rx) -> Recv {
        match rx.try_recv() {
            Ok(v) => Recv::Value(v),
            Err(_) => Recv::Empty,
        }
    }
    fn clone_tx(tx: &Self::Tx) -> Option<Self::Tx> {
        Some(tx.clone())
    }
}

#[cfg(feature = "crossbeam-queue")]
struct CrossbeamArrayQueue;
#[cfg(feature = "crossbeam-queue")]
impl Link for CrossbeamArrayQueue {
    const NAME: &'static str = "crossbeam ArrayQueue";
    type Tx = Arc<crossbeam_queue::ArrayQueue<Stamped>>;
    type Rx = Arc<crossbeam_queue::ArrayQueue<Stamped>>;

    fn create() -> (Self::Tx, Self::Rx) {
        let queue = Arc::new(crossbeam_queue::ArrayQueue::new(CAPACITY));
        (Arc::clone(&queue), queue)
    }
    fn try_send(tx: &mut Self::Tx, msg: Stamped) -> bool {
        tx.push(msg).is_ok()
    }
    fn try_recv(rx: &mut Self::Rx) -> Recv {
        match rx.pop() {
            Some(v) => Recv::Value(v),
            None => Recv::Empty,
        }
    }
    fn clone_tx(tx: &Self::Tx) -> Option<Self::Tx> {
        Some(Arc::clone(tx))
    }
}

// ---------------------------------------------------------------- runner

struct Pass {
    summary: Summary,
    steady: bool,
    /// Receives that found the head reserved by an unpublished producer.
    busy: u64,
    /// Receives that found nothing at all.
    empty: u64,
    /// Sends refused because the ring was full.
    refused: u64,
}

struct ConsumerResult {
    samples: Vec<u64>,
    busy: u64,
    empty: u64,
}

fn tuned(name: String, on: ProcessorId) -> Builder {
    Builder::new()
        .name(name)
        // TimeCritical plus boost disabled is the whole point: a scheduler that
        // lifts our priority mid-run would be measured as a tail we did not cause.
        .priority(ThreadPriority::TimeCritical)
        .boost_policy(BoostPolicy::Disabled)
        .pin_to(on)
}

fn consume<L: Link>(rx: &mut L::Rx, out: &mut [u64]) -> (u64, u64) {
    let mut busy = 0u64;
    let mut empty = 0u64;

    let mut warm = 0usize;
    while warm < WARMUP {
        match L::try_recv(rx) {
            Recv::Value(_) => warm += 1,
            Recv::Busy | Recv::Empty => spin_loop(),
        }
    }

    let mut got = 0usize;
    while got < out.len() {
        match L::try_recv(rx) {
            Recv::Value(msg) => {
                out[got] = clock::now().saturating_sub(msg.due);
                black_box(msg.seq);
                got += 1;
            }
            Recv::Busy => {
                busy += 1;
                spin_loop();
            }
            Recv::Empty => {
                empty += 1;
                spin_loop();
            }
        }
    }

    (busy, empty)
}

/// Runs the measurement with the sample buffer's pages pinned when Windows
/// allows it, and on the plain buffer when it does not.
///
/// The fallback call sits *after* the match on purpose: a match scrutinee
/// temporary lives until the end of the match, and `LockedMut` has a `Drop`, so
/// a call inside the `Err` arm would still count as a second borrow.
fn measure<L: Link>(rx: &mut L::Rx, buffer: &mut [u64]) -> (u64, u64) {
    match wincore::mem::lock_mut(buffer) {
        Ok(mut pinned) => return consume::<L>(rx, &mut pinned),
        Err(source) => {
            eprintln!(
                "[bench] sample buffer not locked ({source}); page faults may appear in the tail"
            );
        }
    }

    consume::<L>(rx, buffer)
}

/// One pass. `cpus` gives both the producer count and where each one is pinned.
/// `rate_hz == None` saturates the link, which measures throughput rather than
/// latency: the queueing delay then dominates every quantile.
///
/// The schedule is split by residue: producer `k` owns every sequence number
/// congruent to `k` modulo the producer count, so the aggregate offered load is
/// `rate_hz` however many threads deliver it.
fn run<L: Link>(cpus: &[ProcessorId], rate_hz: Option<u64>, cycles_per_ns: f64) -> Option<Pass> {
    let n = cpus.len();
    if n == 0 {
        return None;
    }

    let (tx, mut rx) = L::create();
    let (done, results) = sync_channel::<ConsumerResult>(1);
    let refused = Arc::new(AtomicU64::new(0));

    let period = rate_hz
        .map(|hz| (1_000_000_000.0 / hz as f64 * cycles_per_ns) as u64)
        .unwrap_or(0);
    let total = (WARMUP + SAMPLES) as u64;

    // One shared origin, taken before any thread starts and pushed far enough
    // into the future that all of them are spinning by the time it arrives.
    let origin = clock::now() + clock::cycles(STARTUP_GRACE, cycles_per_ns);

    let mut handles: Vec<L::Tx> = Vec::with_capacity(n);
    if n == 1 {
        handles.push(tx);
    } else {
        for _ in 0..n {
            handles.push(L::clone_tx(&tx)?)
        }
        // The template is not a producer; dropping it keeps the producer count
        // honest for links that close on the last drop.
        drop(tx);
    }

    let mut producers = Vec::with_capacity(n);
    for (k, mut tx) in handles.into_iter().enumerate() {
        let refused = Arc::clone(&refused);
        let spawned = tuned(format!("tx{k}-{}", L::NAME), cpus[k]).spawn(move || {
            let mut local_refused = 0u64;
            while clock::now() < origin {
                spin_loop();
            }

            let mut seq = k as u64;
            while seq < total {
                // Open loop. `due` is when this message should have been sent,
                // computed from the schedule, not read after the fact. If the
                // link backs up we fall behind and the delay lands in the
                // sample, which is exactly what coordinated omission hides when
                // you stamp after a blocking send.
                let due = origin + period.saturating_mul(seq);
                while period != 0 && clock::now() < due {
                    spin_loop();
                }

                while !L::try_send(&mut tx, Stamped { due, seq }) {
                    local_refused += 1;
                    spin_loop();
                }
                seq += n as u64;
            }
            refused.fetch_add(local_refused, Ordering::Relaxed);
        });

        match spawned {
            Ok(spawned) => producers.push(spawned),
            Err(source) => {
                eprintln!(
                    "[bench] producer {k} did not start ({source}); skipping this configuration"
                );
                return None;
            }
        }
    }

    let consumer_cpu = ProcessorId::try_new(CONSUMER_CPU as u32, CPU_GROUP)?;
    let consumer = match tuned(format!("rx-{}", L::NAME), consumer_cpu).spawn(move || {
        // Non-zero on purpose: `vec![0u64; N]` lowers to alloc_zeroed and
        // Windows hands back demand-zero pages that are never touched, so the
        // first-touch faults would land in the middle of the run. Every slot is
        // overwritten before being read anyway.
        let mut buffer = vec![u64::MAX; SAMPLES];

        // The slice, not the Vec: `size_of_val` of a `Vec` is its 24-byte
        // header, so locking the Vec would pin the wrong pages.
        let (busy, empty) = measure::<L>(&mut rx, &mut buffer);

        let _ = done.send(ConsumerResult {
            samples: buffer,
            busy,
            empty,
        });
    }) {
        Ok(spawned) => spawned,
        Err(source) => {
            eprintln!("[bench] consumer did not start ({source}); skipping this configuration");
            return None;
        }
    };

    for spawned in &producers {
        for (setting, source) in spawned.config.failures() {
            eprintln!(
                "[bench] producer: {setting} not applied ({source}); numbers are noisier than they should be"
            );
        }
    }
    for (setting, source) in consumer.config.failures() {
        eprintln!(
            "[bench] consumer: {setting} not applied ({source}); numbers are noisier than they should be"
        );
    }

    let result = results.recv().ok();
    for spawned in producers {
        let _ = spawned.handle.join();
    }
    let _ = consumer.handle.join();

    let mut result = result?;
    // Before `summarize`, which sorts the samples out of arrival order.
    let steady = kept_up(&result.samples);
    let summary = summarize(&mut result.samples, cycles_per_ns)?;

    Some(Pass {
        summary,
        steady,
        busy: result.busy,
        empty: result.empty,
        refused: refused.load(Ordering::Relaxed),
    })
}

// ---------------------------------------------------------------- setup

fn raise_working_set() {
    use wincore::process::{WorkingSetLimits, modify_working_set_limits, working_set_limits};

    let current = match working_set_limits() {
        Ok(limits) => limits,
        Err(source) => {
            eprintln!("[bench] cannot read the working set limits: {source}");
            return;
        }
    };

    let wanted = WorkingSetLimits {
        minimum: current.minimum.max(WANTED_WORKING_SET_MIN),
        maximum: current.maximum.max(WANTED_WORKING_SET_MAX),
        flags: Vec::new(),
    };

    if let Err(source) = modify_working_set_limits(wanted) {
        eprintln!(
            "[bench] working set not raised ({source}); the sample buffer will not be locked"
        );
    }
}

/// Turns a CPU list into pinned processor ids, refusing anything that would
/// silently measure something other than what the table claims.
fn resolve(cpus: &[u8], label: &str) -> Option<Vec<ProcessorId>> {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);

    let mut resolved = Vec::with_capacity(cpus.len());
    for &cpu in cpus {
        if logical != 0 && cpu as usize >= logical {
            eprintln!(
                "[bench] {label}: CPU {cpu} does not exist on this machine ({logical} logical CPUs); table skipped"
            );
            return None;
        }
        if cpu == CONSUMER_CPU {
            eprintln!(
                "[bench] {label}: CPU {cpu} is the consumer's; a producer sharing it would measure the wrong thing; table skipped"
            );
            return None;
        }
        match ProcessorId::try_new(cpu as u32, CPU_GROUP) {
            Some(id) => resolved.push(id),
            None => {
                eprintln!(
                    "[bench] {label}: CPU {cpu} is beyond the affinity mask width; table skipped"
                );
                return None;
            }
        }
    }
    Some(resolved)
}

/// Warns when a CPU list contradicts the assumption its name makes. Sibling
/// logical CPUs are `2k` and `2k+1` on the usual enumeration, so an "exclusive"
/// list must hold at most one CPU per pair.
fn check_topology() {
    let pairs: Vec<u8> = CORES_EXCLUSIVE.iter().map(|c| c / 2).collect();
    for (i, a) in pairs.iter().enumerate() {
        if pairs[i + 1..].contains(a) {
            eprintln!(
                "[bench] WARNING: CORES_EXCLUSIVE {CORES_EXCLUSIVE:?} puts two producers on the \
                 hyper-threads of one core. The fan-in table then measures pipeline sharing, \
                 not the queue."
            );
            break;
        }
    }
    if CORES_SIBLING_PAIR[0] / 2 != CORES_SIBLING_PAIR[1] / 2 {
        eprintln!(
            "[bench] WARNING: CORES_SIBLING_PAIR {CORES_SIBLING_PAIR:?} are not two hyper-threads \
             of the same core; the control experiment has no control."
        );
    }
}

// ---------------------------------------------------------------- reporting

const COLUMNS: [&str; 6] = ["min", "p50", "p90", "p99", "p99.9", "max"];

fn header(title: &str) {
    println!("\n{title}");
    println!(
        "{:<26} {:>8} {:>8} {:>8} {:>8} {:>9} {:>10} {:>9} {:>9}",
        "link",
        COLUMNS[0],
        COLUMNS[1],
        COLUMNS[2],
        COLUMNS[3],
        COLUMNS[4],
        COLUMNS[5],
        "busy",
        "refused"
    );
    println!(
        "{:-<26} {:->8} {:->8} {:->8} {:->8} {:->9} {:->10} {:->9} {:->9}",
        "", "", "", "", "", "", "", "", ""
    );
}

/// Runs `PASSES + 1` passes, discards the first, prints the median of the rest
/// and emits one `#DATA,` line per measured pass.
fn report<L: Link>(scenario: &str, cpus: &[ProcessorId], rate_hz: Option<u64>, cycles_per_ns: f64) {
    let label = match cpus.len() {
        1 => L::NAME.to_string(),
        n => format!("{} x{n}", L::NAME),
    };

    // The discarded pass lets the cores ramp up and the allocator settle.
    if run::<L>(cpus, rate_hz, cycles_per_ns).is_none() {
        println!("{label:<26} skipped (this link takes a single producer only)");
        return;
    }

    let mut passes = Vec::with_capacity(PASSES);
    for _ in 0..PASSES {
        match run::<L>(cpus, rate_hz, cycles_per_ns) {
            Some(pass) => passes.push(pass),
            None => break,
        }
    }
    if passes.is_empty() {
        println!("{label:<26} no pass completed");
        return;
    }

    for (index, pass) in passes.iter().enumerate() {
        println!(
            "#DATA,{scenario},{name},{producers},{rate},{index},{min:.3},{p50:.3},{p90:.3},{p99:.3},{p999:.3},{max:.3},{steady},{busy},{empty},{refused},{count}",
            name = L::NAME,
            producers = cpus.len(),
            rate = rate_hz.unwrap_or(0),
            min = pass.summary.min,
            p50 = pass.summary.p50,
            p90 = pass.summary.p90,
            p99 = pass.summary.p99,
            p999 = pass.summary.p999,
            max = pass.summary.max,
            steady = u8::from(pass.steady),
            busy = pass.busy,
            empty = pass.empty,
            refused = pass.refused,
            count = pass.summary.count,
        );
    }

    let mut medians = [0.0f64; 6];
    for (index, slot) in medians.iter_mut().enumerate() {
        let mut values: Vec<f64> = passes.iter().map(|p| p.summary.field(index)).collect();
        *slot = median(&mut values);
    }
    let mut busy: Vec<f64> = passes.iter().map(|p| p.busy as f64).collect();
    let mut refused: Vec<f64> = passes.iter().map(|p| p.refused as f64).collect();
    let unsteady = passes.iter().filter(|p| !p.steady).count();

    println!(
        "{:<26} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>9.1} {:>10.1} {:>9.0} {:>9.0}{}",
        label,
        medians[0],
        medians[1],
        medians[2],
        medians[3],
        medians[4],
        medians[5],
        median(&mut busy),
        median(&mut refused),
        match unsteady {
            0 => String::new(),
            n => format!(
                "   {n}/{} passes did not keep up: those are backlog, not latency",
                passes.len()
            ),
        }
    );
}

/// Every multi-producer link, at one producer count.
fn report_fan_in(scenario: &str, cpus: &[ProcessorId], rate: Option<u64>, cycles_per_ns: f64) {
    report::<Mpsc>(scenario, cpus, rate, cycles_per_ns);
    #[cfg(feature = "crossbeam-queue")]
    report::<CrossbeamArrayQueue>(scenario, cpus, rate, cycles_per_ns);
    #[cfg(feature = "crossbeam-channel")]
    report::<CrossbeamChannel>(scenario, cpus, rate, cycles_per_ns);
    report::<StdMpsc>(scenario, cpus, rate, cycles_per_ns);
}

fn main() {
    raise_working_set();
    check_topology();
    clock::check_cross_core();

    let cycles_per_ns = clock::calibrate();
    if !PLAUSIBLE_CYCLES_PER_NS.contains(&cycles_per_ns) {
        eprintln!(
            "[bench] calibration returned {cycles_per_ns} cycles/ns, which is not plausible; aborting"
        );
        std::process::exit(1);
    }
    let overhead = clock::overhead_cycles() as f64 / cycles_per_ns;

    println!(
        "TSC: {cycles_per_ns:.3} cycles/ns, one read costs ~{overhead:.1} ns (not subtracted below)"
    );
    println!(
        "{SAMPLES} samples per pass, {WARMUP} discarded, {PASSES} measured passes plus one thrown away, capacity {CAPACITY}"
    );
    println!(
        "Consumer on CPU {CONSUMER_CPU}. Nanoseconds, one way. Rows are the median across passes."
    );
    println!("Read the tail, not the median: the clock read above costs more than the operation.");
    println!(
        "'busy' counts receives that found the head reserved by an unpublished producer; 'refused' counts full-ring sends."
    );

    let Some(solo) = resolve(&CORES_EXCLUSIVE[..1], "single producer") else {
        return;
    };

    for rate in RATES_HZ {
        header(&format!("One producer, {rate} msg/s offered"));
        report::<Spsc>("paced", &solo, Some(rate), cycles_per_ns);
        report_fan_in("paced", &solo, Some(rate), cycles_per_ns);
    }

    for n in FAN_IN {
        let Some(cpus) = resolve(&CORES_EXCLUSIVE[..n], "fan-in") else {
            continue;
        };
        header(&format!(
            "Fan-in x{n}, one physical core each (CPUs {:?}), {FAN_IN_RATE} msg/s offered in total",
            &CORES_EXCLUSIVE[..n]
        ));
        report_fan_in("fan_in", &cpus, Some(FAN_IN_RATE), cycles_per_ns);
    }

    // Control experiment: the only difference between these two tables is
    // whether the two producers share a core's L1 and pipeline.
    if let Some(cpus) = resolve(&CORES_DISTINCT_PAIR, "distinct pair") {
        header(&format!(
            "Control A: 2 producers on 2 physical cores (CPUs {CORES_DISTINCT_PAIR:?}), {FAN_IN_RATE} msg/s offered in total"
        ));
        report_fan_in("pair_distinct", &cpus, Some(FAN_IN_RATE), cycles_per_ns);
    }
    if let Some(cpus) = resolve(&CORES_SIBLING_PAIR, "sibling pair") {
        header(&format!(
            "Control B: 2 producers on 1 physical core, both hyper-threads (CPUs {CORES_SIBLING_PAIR:?}), {FAN_IN_RATE} msg/s offered in total"
        ));
        report_fan_in("pair_sibling", &cpus, Some(FAN_IN_RATE), cycles_per_ns);
    }

    if let Some(cpus) = resolve(&CORES_OVERSUBSCRIBED, "oversubscribed") {
        header(&format!(
            "Oversubscribed: {} producers on {} physical cores, both hyper-threads (CPUs {CORES_OVERSUBSCRIBED:?}), {FAN_IN_RATE} msg/s offered in total",
            CORES_OVERSUBSCRIBED.len(),
            CORES_OVERSUBSCRIBED.len() / 2,
        ));
        report_fan_in("oversub", &cpus, Some(FAN_IN_RATE), cycles_per_ns);
    }

    header(
        "Saturated: no pacing. Every figure is queueing delay; throughput is SAMPLES+WARMUP divided by max.",
    );
    report::<Spsc>("saturated", &solo, None, cycles_per_ns);
    for n in FAN_IN {
        let Some(cpus) = resolve(&CORES_EXCLUSIVE[..n], "saturated fan-in") else {
            continue;
        };
        report::<Mpsc>("saturated", &cpus, None, cycles_per_ns);
    }
    if let Some(cpus) = resolve(&CORES_OVERSUBSCRIBED, "saturated oversubscribed") {
        report::<Mpsc>("saturated", &cpus, None, cycles_per_ns);
    }
}
