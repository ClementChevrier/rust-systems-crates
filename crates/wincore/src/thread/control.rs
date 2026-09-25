//! Changing a thread's placement and priority after it started.
//!
//! Each setting comes in two forms: `*_current_*` for the calling thread, and a
//! form taking a `JoinHandle`, whose handle is valid for as long as the
//! `JoinHandle` is borrowed.

use std::{ffi::c_void, io, os::windows::io::AsRawHandle, thread::JoinHandle};

use crate::ffi::*;

use super::{affinity::*, priority::*};

fn inner_pin_thread_to_cpu(handle: HANDLE, cpu: ProcessorId) -> Result<(), io::Error> {
    if cpu.number as u32 >= usize::BITS {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }

    let affinity = C_GroupAffinity::new(cpu);
    // SAFETY: `handle` is a live thread handle (see the callers), `affinity` a
    // live `GROUP_AFFINITY`, and a null previous-affinity pointer is allowed.
    let rslt = unsafe {
        SetThreadGroupAffinity(
            handle,
            &affinity as *const _ as *const c_void,
            std::ptr::null_mut(),
        )
    };

    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Pins the thread behind `handle` to a single logical CPU.
pub fn pin_thread_to_cpu<T>(handle: &JoinHandle<T>, cpu: ProcessorId) -> Result<(), io::Error> {
    inner_pin_thread_to_cpu(handle.as_raw_handle(), cpu)
}

/// Pins the calling thread to a single logical CPU.
pub fn pin_current_thread_to_cpu(cpu: ProcessorId) -> Result<(), io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle to the calling thread.
    inner_pin_thread_to_cpu(unsafe { GetCurrentThread() }, cpu)
}

fn inner_set_thread_priority(handle: HANDLE, prio: ThreadPriority) -> Result<(), io::Error> {
    // SAFETY: `handle` is a live thread handle (see the callers).
    if unsafe { SetThreadPriority(handle, prio.as_win32()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Sets the priority level of the calling thread.
pub fn set_current_priority(prio: ThreadPriority) -> Result<(), io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle to the calling thread.
    inner_set_thread_priority(unsafe { GetCurrentThread() }, prio)
}

/// Sets the priority level of the thread behind `handle`.
pub fn set_priority<T>(handle: &JoinHandle<T>, prio: ThreadPriority) -> Result<(), io::Error> {
    inner_set_thread_priority(handle.as_raw_handle(), prio)
}

fn inner_set_boost_policy(handle: HANDLE, policy: BoostPolicy) -> Result<(), io::Error> {
    // SAFETY: `handle` is a live thread handle (see the callers).
    if unsafe { SetThreadPriorityBoost(handle, policy.as_win32()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Sets the boost policy of the calling thread.
pub fn set_current_boost_policy(policy: BoostPolicy) -> Result<(), io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle to the calling thread.
    inner_set_boost_policy(unsafe { GetCurrentThread() }, policy)
}

/// Sets the boost policy of the thread behind `handle`.
pub fn set_boost_policy<T>(handle: &JoinHandle<T>, policy: BoostPolicy) -> Result<(), io::Error> {
    inner_set_boost_policy(handle.as_raw_handle(), policy)
}
