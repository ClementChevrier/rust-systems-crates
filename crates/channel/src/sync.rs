#[cfg(loom)]
pub(crate) use loom::{
    cell,
    sync::{Arc, atomic},
};

#[cfg(not(loom))]
pub(crate) use std::sync::{Arc, atomic};

#[cfg(not(loom))]
pub(crate) mod cell {
    //! `std::cell::UnsafeCell` behind loom's closure-based API, so the same
    //! code compiles against both.
    //! See <https://docs.rs/loom/latest/loom/#handling-loom-api-differences>.

    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        pub(crate) fn new(data: T) -> UnsafeCell<T> {
            UnsafeCell(std::cell::UnsafeCell::new(data))
        }

        pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
            f(self.0.get())
        }

        pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
            f(self.0.get())
        }
    }
}
