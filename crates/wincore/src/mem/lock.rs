//! Pinning buffers into physical memory.
//!
//! # Safety
//!
//! `VirtualLock` and `VirtualUnlock` only change how the pages behind an
//! address range are paged; they never read or write the memory. Each call
//! below passes the address and `size_of_val` of a value borrowed for the
//! whole life of the guard, so the range is always a live allocation.

use std::{ffi::c_void, io::Error};

use crate::ffi::{VirtualLock, VirtualUnlock};

/// Locks the pages backing `value` into RAM for the lifetime of the returned guard.
///
/// Those pages can no longer be evicted to the page file, so a later access never
/// costs a hard page fault.
///
/// Locking has page granularity, not value granularity, and Windows does not
/// reference-count it: if two locked values land in the same page, dropping the
/// first guard unlocks the page for both. Only lock buffers whose placement you
/// control.
///
/// Pass the data, not its owner: `lock(&vec)` locks the `Vec` header on the
/// stack, `lock(vec.as_slice())` locks the elements.
///
/// Windows caps how much a process may lock, at about the minimum working set
/// (see [`crate::process::modify_working_set_limits`]); past that, the call
/// fails.
pub fn lock<T: ?Sized>(value: &T) -> Result<Locked<'_, T>, Error> {
    Locked::try_new(value)
}

/// Like [`lock`], granting mutable access to the locked value.
pub fn lock_mut<T: ?Sized>(value: &mut T) -> Result<LockedMut<'_, T>, Error> {
    LockedMut::try_new(value)
}

/// Locks `size` bytes from `address`. A zero-sized value has no page to lock.
fn lock_range(address: *mut c_void, size: usize) -> Result<(), Error> {
    if size == 0 {
        return Ok(());
    }
    // SAFETY: see the module docs: the range is a live, borrowed value.
    if unsafe { VirtualLock(address, size) } == 0 {
        Err(Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Unlocks what `lock_range` locked.
///
/// The result is ignored because `Drop` cannot report an error. `VirtualUnlock`
/// fails with `ERROR_NOT_LOCKED` when a page in the range is already unlocked,
/// for example when two guards share a page. That never invalidates the
/// memory: the page merely becomes evictable again.
fn unlock_range(address: *mut c_void, size: usize) {
    if size == 0 {
        return;
    }
    // SAFETY: see the module docs: the range is a live, borrowed value.
    let _ = unsafe { VirtualUnlock(address, size) };
}

/// Guard returned by [`lock`]: the pages are unlocked when it drops.
pub struct Locked<'a, T: ?Sized> {
    value: &'a T,
    size: usize,
}

impl<'a, T: ?Sized> Locked<'a, T> {
    fn try_new(value: &'a T) -> Result<Self, Error> {
        let size = std::mem::size_of_val(value);
        lock_range(value as *const T as *mut c_void, size)?;
        Ok(Self { value, size })
    }
}

impl<T: ?Sized> Drop for Locked<'_, T> {
    fn drop(&mut self) {
        unlock_range(self.value as *const T as *mut c_void, self.size);
    }
}

impl<T: ?Sized> std::ops::Deref for Locked<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

/// Guard returned by [`lock_mut`]: the pages are unlocked when it drops.
pub struct LockedMut<'a, T: ?Sized> {
    value: &'a mut T,
    size: usize,
}

impl<'a, T: ?Sized> LockedMut<'a, T> {
    fn try_new(value: &'a mut T) -> Result<Self, Error> {
        let size = std::mem::size_of_val(value);
        lock_range(value as *mut T as *mut c_void, size)?;
        Ok(Self { value, size })
    }
}

impl<T: ?Sized> Drop for LockedMut<'_, T> {
    fn drop(&mut self) {
        unlock_range(self.value as *mut T as *mut c_void, self.size);
    }
}

impl<T: ?Sized> std::ops::Deref for LockedMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T: ?Sized> std::ops::DerefMut for LockedMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

#[cfg(test)]
#[path = "tests/lock.rs"]
mod lock_test;
