//! Hand-written kernel32 declarations: only the calls this crate makes, with
//! the Win32 names and types so each one can be checked against its MSDN page.
//!
//! Every pointer parameter is typed by what it points to in Win32; the Rust
//! side passes a pointer to a live local of the matching `repr(C)` layout.

#![allow(clippy::upper_case_acronyms, non_camel_case_types, non_snake_case)]

use std::os::raw::c_void;

type BOOL = i32;
type PBOOL = *mut i32;

type DWORD = u32;
type PDWORD = *mut u32;

type SIZE_T = usize;
type PSIZE_T = *mut usize;

type PULONG64 = *mut u64;

type LPHANDLE = *mut HANDLE;
type LPVOID = *mut c_void;
type LPFILETIME = *mut c_void;

type PPROCESS_MEMORY_COUNTERS = *mut c_void;
type PPROCESSOR_NUMBER = *mut c_void;
type PGROUP_AFFINITY = *mut c_void;

pub(crate) type HANDLE = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    pub(crate) fn GetCurrentThread() -> HANDLE;
    pub(crate) fn GetCurrentProcess() -> HANDLE;
    #[cfg(test)]
    pub(crate) fn GetCurrentThreadId() -> DWORD;

    pub(crate) fn SetThreadPriority(hThread: HANDLE, nPriority: i32) -> BOOL;

    pub(crate) fn SetThreadPriorityBoost(hThread: HANDLE, bDisablePriorityBoost: BOOL) -> BOOL;

    pub(crate) fn SetThreadGroupAffinity(
        hThread: HANDLE,
        GroupAffinity: *const c_void,
        PreviousGroupAffinity: PGROUP_AFFINITY,
    ) -> BOOL;

    pub(crate) fn SetPriorityClass(hProcess: HANDLE, dwPriorityClass: DWORD) -> BOOL;

    pub(crate) fn GetPriorityClass(hProcess: HANDLE) -> DWORD;

    pub(crate) fn VirtualLock(lpAddress: LPVOID, dwSize: SIZE_T) -> BOOL;

    pub(crate) fn VirtualUnlock(lpAddress: LPVOID, dwSize: SIZE_T) -> BOOL;

    pub(crate) fn K32GetProcessMemoryInfo(
        Process: HANDLE,
        ppsmemCounters: PPROCESS_MEMORY_COUNTERS,
        cb: DWORD,
    ) -> BOOL;

    pub(crate) fn SetProcessWorkingSetSizeEx(
        hProcess: HANDLE,
        dwMinimumWorkingSetSize: SIZE_T,
        dwMaximumWorkingSetSize: SIZE_T,
        Flags: DWORD,
    ) -> BOOL;

    pub(crate) fn GetProcessWorkingSetSizeEx(
        hProcess: HANDLE,
        lpMinimumWorkingSetSize: PSIZE_T,
        lpMaximumWorkingSetSize: PSIZE_T,
        Flags: PDWORD,
    ) -> BOOL;

    pub(crate) fn GetThreadTimes(
        hThread: HANDLE,
        lpCreationTime: LPFILETIME,
        lpExitTime: LPFILETIME,
        lpKernelTime: LPFILETIME,
        lpUserTime: LPFILETIME,
    ) -> BOOL;

    pub(crate) fn QueryThreadCycleTime(ThreadHandle: HANDLE, CycleTime: PULONG64) -> BOOL;

    pub(crate) fn GetThreadPriority(hThread: HANDLE) -> i32;

    pub(crate) fn GetThreadPriorityBoost(hThread: HANDLE, pDisablePriorityBoost: PBOOL) -> BOOL;

    pub(crate) fn GetThreadIdealProcessorEx(
        hThread: HANDLE,
        lpIdealProcessor: PPROCESSOR_NUMBER,
    ) -> BOOL;

    pub(crate) fn GetThreadGroupAffinity(hThread: HANDLE, GroupAffinity: PGROUP_AFFINITY) -> BOOL;

    pub(crate) fn GetThreadId(Thread: HANDLE) -> DWORD;

    pub(crate) fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;

    pub(crate) fn DuplicateHandle(
        hSourceProcessHandle: HANDLE,
        hSourceHandle: HANDLE,
        hTargetProcessHandle: HANDLE,
        lpTargetHandle: LPHANDLE,
        dwDesiredAccess: DWORD,
        bInheritHandle: BOOL,
        dwOptions: DWORD,
    ) -> BOOL;

    pub(crate) fn CloseHandle(hObject: HANDLE) -> BOOL;
}
