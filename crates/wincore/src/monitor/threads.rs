use std::{
    io,
    os::windows::io::AsRawHandle,
    sync::{Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
};

use crate::{
    ffi::*,
    thread::{
        ConfigReport,
        affinity::{ProcessorId, ThreadAffinity},
        priority::*,
        query,
        times::ThreadTimes,
    },
};

pub(crate) static REGISTRY: Mutex<Vec<ThreadEntry>> = Mutex::new(Vec::new());
pub(crate) fn registry_lock() -> MutexGuard<'static, Vec<ThreadEntry>> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A real thread handle owned by the registry, closed on drop.
#[derive(Debug)]
pub(crate) struct DuplicatedHandle(HANDLE);
impl DuplicatedHandle {
    fn as_raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for DuplicatedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from `DuplicateHandle`, is owned by this
        // value alone, and is closed exactly once, here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
// SAFETY: a Win32 handle is a process-wide table index, valid from any thread;
// this wrapper holds no thread-local state.
unsafe impl Send for DuplicatedHandle {}

const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;
pub(crate) fn duplicate_thread_handle(handle: HANDLE) -> Result<DuplicatedHandle, io::Error> {
    let mut duplicated: HANDLE = std::ptr::null_mut();
    // SAFETY: `handle` is a live thread handle (see the callers), the process
    // pseudo-handles need no closing, and the out-pointer points at a live local.
    let rslt = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut duplicated,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };

    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(DuplicatedHandle(duplicated))
    }
}

/// The configuration requested for a thread, kept next to what Windows reports.
///
/// A `None` field means "not requested": the thread keeps the Windows default.
#[derive(Debug, Default, Clone)]
pub struct ThreadConfig {
    /// Requested priority level.
    pub prio: Option<ThreadPriority>,
    /// Requested boost policy.
    pub prio_boost: Option<BoostPolicy>,
    /// Requested affinity.
    pub affinity: Option<ThreadAffinity>,
}
impl From<ConfigReport> for ThreadConfig {
    fn from(value: ConfigReport) -> Self {
        ThreadConfig {
            prio: value.prio.wanted().copied(),
            prio_boost: value.boost.wanted().copied(),
            affinity: value.pin.wanted().copied().map(ThreadAffinity::single_cpu),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ThreadEntry {
    pub(crate) handle: DuplicatedHandle,
    // Windows thread id, not Rust's `ThreadId`.
    pub(crate) tid: u32,
    pub(crate) name: String,
    pub(crate) config: ThreadConfig,
}

/// Adds an entry unless its thread is already enrolled, dropping dead entries
/// on the way.
fn enrol(entry: ThreadEntry) {
    let mut reg = registry_lock();
    reg.retain(|e| !matches!(query::is_alive(e.handle.as_raw()), Ok(false)));
    if reg.iter().any(|e| e.tid == entry.tid) {
        return;
    }
    reg.push(entry);
    reg.sort_unstable_by_key(|e| e.tid);
}

/// Enrols the calling thread in the monitoring registry.
///
/// Duplicates the current thread's pseudo-handle into a real one: otherwise the
/// stored value would designate "whichever thread is reading" rather than the
/// one being enrolled. The handle stays open for as long as the entry lives,
/// which keeps the thread's statistics readable even after it has died.
///
/// Enrolling a thread twice keeps the first entry.
pub fn watch_current(config: ThreadConfig) -> Result<(), io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle to the calling thread.
    let handle = duplicate_thread_handle(unsafe { GetCurrentThread() })?;
    let tid = query::tid(handle.as_raw())?;
    let name = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_string();

    enrol(ThreadEntry {
        handle,
        tid,
        name,
        config,
    });
    Ok(())
}

/// Enrols the thread behind `handle` in the monitoring registry.
///
/// The registry keeps its own duplicate of the handle, so the entry stays
/// valid after the `JoinHandle` is joined or dropped.
///
/// Enrolling a thread twice keeps the first entry.
pub fn watch<T>(handle: &JoinHandle<T>, config: ThreadConfig) -> Result<(), io::Error> {
    let raw_handle = handle.as_raw_handle();
    let duplicated = duplicate_thread_handle(raw_handle)?;
    let tid = query::tid(raw_handle)?;
    let name = handle.thread().name().unwrap_or("unnamed").to_string();

    enrol(ThreadEntry {
        handle: duplicated,
        tid,
        name,
        config,
    });
    Ok(())
}

/// Removes the thread behind `handle` from the registry.
pub fn unwatch<T>(handle: &JoinHandle<T>) -> Result<(), io::Error> {
    let tid = query::tid(handle.as_raw_handle())?;
    registry_lock().retain(|t| t.tid != tid);
    Ok(())
}

/// Removes the calling thread from the registry.
pub fn unwatch_current() -> Result<(), io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle to the calling thread.
    let tid = query::tid(unsafe { GetCurrentThread() })?;
    registry_lock().retain(|t| t.tid != tid);
    Ok(())
}

/// Drops the entries of threads that have exited. Entries whose state cannot
/// be read are kept.
pub fn purge_dead() {
    registry_lock().retain(|e| !matches!(query::is_alive(e.handle.as_raw()), Ok(false)));
}

/// Number of entries in the registry, dead threads included until they are purged.
pub fn watched_count() -> usize {
    registry_lock().len()
}

/// One enrolled thread in a [`Snapshot`](super::Snapshot).
#[derive(Debug)]
pub struct ThreadReport {
    /// Windows thread id.
    pub tid: u32,
    /// Rust thread name at enrolment, or `unnamed`.
    pub name: String,
    /// Whether the thread was still running.
    pub alive: Result<bool, io::Error>,

    /// What was asked for.
    pub config: ThreadConfig,
    /// What Windows reports.
    pub observed: ThreadState,
}
impl ThreadReport {
    pub(crate) fn capture(entry: &ThreadEntry) -> Self {
        Self {
            tid: entry.tid,
            name: entry.name.clone(),
            alive: query::is_alive(entry.handle.as_raw()),

            config: entry.config.clone(),
            observed: ThreadState::capture(entry),
        }
    }
}

/// A thread's state as Windows reports it. Each field is a separate query, so
/// one refusal does not hide the others.
#[derive(Debug)]
pub struct ThreadState {
    /// Creation time and CPU time.
    pub times: Result<ThreadTimes, io::Error>,
    /// CPU cycles consumed.
    pub cycle_count: Result<u64, io::Error>,

    /// Priority level.
    pub priority: Result<RawThreadPriority, io::Error>,
    /// Boost policy.
    pub priority_boost: Result<BoostPolicy, io::Error>,

    /// Allowed CPUs.
    pub affinity: Result<ThreadAffinity, io::Error>,
    /// Preferred CPU.
    pub ideal_processor: Result<ProcessorId, io::Error>,
}
impl ThreadState {
    fn capture(entry: &ThreadEntry) -> Self {
        let raw_handle = entry.handle.as_raw();
        Self {
            times: query::times(raw_handle),
            cycle_count: query::cycles_count(raw_handle),

            priority: query::priority(raw_handle),
            priority_boost: query::boost_policy(raw_handle),

            affinity: query::affinity(raw_handle),
            ideal_processor: query::ideal_processor(raw_handle),
        }
    }

    /// The queries Windows refused, named, for a report that says *what*
    /// failed rather than just showing a dash.
    pub fn failures(&self) -> impl Iterator<Item = (&'static str, &io::Error)> {
        [
            ("times", self.times.as_ref().err()),
            ("cycle_count", self.cycle_count.as_ref().err()),
            ("priority", self.priority.as_ref().err()),
            ("priority_boost", self.priority_boost.as_ref().err()),
            ("affinity", self.affinity.as_ref().err()),
            ("ideal_processor", self.ideal_processor.as_ref().err()),
        ]
        .into_iter()
        .filter_map(|(what, source)| source.map(|source| (what, source)))
    }

    /// True when every query succeeded.
    pub fn is_complete(&self) -> bool {
        self.failures().next().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_records_the_watched_thread_name_not_the_caller() {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("watched-worker".to_string())
            .spawn(move || {
                let _ = release_rx.recv();
            })
            .expect("spawn");

        watch(&handle, ThreadConfig::default()).expect("watch");
        let tid = query::tid(handle.as_raw_handle()).expect("tid");
        let name = registry_lock()
            .iter()
            .find(|entry| entry.tid == tid)
            .map(|entry| entry.name.clone());

        release_tx.send(()).expect("release");
        handle.join().expect("thread must not panic");
        purge_dead();
        assert_eq!(name.as_deref(), Some("watched-worker"));
    }
}
