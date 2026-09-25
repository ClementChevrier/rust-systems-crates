//! Reading a thread's real state, as Windows reports it.
//!
//! Every function here takes a raw `HANDLE`. None is marked `unsafe`: a Win32
//! handle is an index into the process handle table, not a dereferenced pointer,
//! and the kernel rejects invalid values with `ERROR_INVALID_HANDLE` rather than
//! corrupting anything. The out-pointers passed to Windows always point at live
//! locals of the matching `repr(C)` layout.
//!
//! Two conditions stay the caller's responsibility, and neither is checkable by
//! the compiler:
//!
//! - the handle must stay **open for the whole duration of the call**. Closing it
//!   from another thread meanwhile makes the query land on an arbitrary kernel
//!   object, because Windows recycles handle values;
//! - it must grant at least `THREAD_QUERY_INFORMATION`, plus `SYNCHRONIZE` for
//!   [`is_alive`].
//!
//! A handle taken from a `JoinHandle` satisfies both for as long as that
//! `JoinHandle` is alive; it is closed by `join` and by `Drop`.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::{ffi::c_void, io};

use super::{
    affinity::*,
    priority::{BoostPolicy, RawThreadPriority},
    times::*,
};
use crate::ffi::*;

/// The thread's timestamps and CPU time (`GetThreadTimes`).
pub fn times(t_handle: HANDLE) -> Result<ThreadTimes, io::Error> {
    let mut creation = C_FileTime::default();
    let mut exit = C_FileTime::default();
    let mut kernel = C_FileTime::default();
    let mut user = C_FileTime::default();

    // SAFETY: the four out-pointers point at live `FILETIME` locals.
    let rslt = unsafe {
        GetThreadTimes(
            t_handle,
            &mut creation as *mut _ as *mut c_void,
            &mut exit as *mut _ as *mut c_void,
            &mut kernel as *mut _ as *mut c_void,
            &mut user as *mut _ as *mut c_void,
        )
    };

    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ThreadTimes {
            creation: creation.into_system_time(),
            kernel: kernel.into_duration(),
            user: user.into_duration(),
        })
    }
}

/// CPU cycles consumed since the thread was created (`QueryThreadCycleTime`).
///
/// Finer than CPU time for comparing two threads: it is not quantised by the
/// scheduler tick.
pub fn cycles_count(t_handle: HANDLE) -> Result<u64, io::Error> {
    let mut cycle = 0u64;
    // SAFETY: the out-pointer points at a live `u64`.
    if unsafe { QueryThreadCycleTime(t_handle, &mut cycle) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(cycle)
    }
}

const THREAD_PRIORITY_ERROR_RETURN: i32 = 0x7FFF_FFFF;

/// The thread's priority level (`GetThreadPriority`), relative to its process
/// class.
pub fn priority(t_handle: HANDLE) -> Result<RawThreadPriority, io::Error> {
    // SAFETY: plain value call; the kernel validates the handle.
    let prio = unsafe { GetThreadPriority(t_handle) };

    if prio == THREAD_PRIORITY_ERROR_RETURN {
        Err(io::Error::last_os_error())
    } else {
        Ok(RawThreadPriority::new(prio))
    }
}

/// The thread's current boost policy (`GetThreadPriorityBoost`).
pub fn boost_policy(t_handle: HANDLE) -> Result<BoostPolicy, io::Error> {
    let mut disabled = 0i32;
    // SAFETY: the out-pointer points at a live `BOOL`.
    if unsafe { GetThreadPriorityBoost(t_handle, &mut disabled) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(BoostPolicy::from_win32(disabled))
    }
}

/// The CPU Windows prefers to schedule this thread on (`GetThreadIdealProcessorEx`).
///
/// A scheduling hint, not a constraint: unlike [`affinity`], it forbids no CPU.
pub fn ideal_processor(t_handle: HANDLE) -> Result<ProcessorId, io::Error> {
    let mut ideal = C_ProcessorNumber::default();
    // SAFETY: the out-pointer points at a live `PROCESSOR_NUMBER`.
    let rslt = unsafe { GetThreadIdealProcessorEx(t_handle, &mut ideal as *mut _ as *mut c_void) };
    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ideal.into())
    }
}

/// The CPUs the thread is allowed to run on (`GetThreadGroupAffinity`).
pub fn affinity(t_handle: HANDLE) -> Result<ThreadAffinity, io::Error> {
    let mut group = C_GroupAffinity::default();
    // SAFETY: the out-pointer points at a live `GROUP_AFFINITY`.
    if unsafe { GetThreadGroupAffinity(t_handle, &mut group as *mut _ as *mut c_void) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(group.into())
    }
}

/// The thread's Windows identifier (`GetThreadId`).
///
/// This is the TID shown by Process Explorer and ETW traces, unrelated to Rust's
/// `std::thread::ThreadId`, which is process-local.
pub fn tid(t_handle: HANDLE) -> Result<u32, io::Error> {
    // SAFETY: plain value call; the kernel validates the handle.
    let id = unsafe { GetThreadId(t_handle) };

    if id == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(id)
    }
}

const WAIT_OBJECT_0: u32 = 0x0000_0000;
const WAIT_TIMEOUT: u32 = 0x0000_0102;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;

/// Whether the thread is still running.
///
/// A thread handle becomes signalled when the thread exits; this tests that state
/// without waiting. The handle stays queryable after the thread has died, which is
/// what keeps its final statistics readable.
pub fn is_alive(t_handle: HANDLE) -> Result<bool, io::Error> {
    // A zero timeout makes the wait a non-blocking check.
    // SAFETY: plain value call; the kernel validates the handle.
    match unsafe { WaitForSingleObject(t_handle, 0) } {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => Ok(true),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        // WAIT_ABANDONED only exists for mutexes; a thread handle never
        // returns it. Report rather than panic inside a monitoring call.
        other => Err(io::Error::other(format!(
            "unexpected WaitForSingleObject result {other:#x} on a thread handle"
        ))),
    }
}

#[cfg(test)]
#[path = "tests/query.rs"]
mod query_test;
