// IA says that on x86_64/aarch64 when a line is fetch into the cache it's loaded
// by paris. Hence the padding must be 128 instead of 64. Used a bit more memory but it's
// okay.

// I bench it and result are similare hence i keep it to 128 has a bit of lost memory is not a problem
// for the moment!
#[cfg_attr(channel_pad = "64", repr(align(64)))]
#[cfg_attr(channel_pad = "128", repr(align(128)))]
#[cfg_attr(
    all(
        not(channel_pad = "64"),
        not(channel_pad = "128"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    repr(align(128))
)]
#[cfg_attr(
    all(
        not(channel_pad = "64"),
        not(channel_pad = "128"),
        not(any(target_arch = "x86_64", target_arch = "aarch64"))
    ),
    repr(align(64))
)]
pub(crate) struct CachePadded<T>(T);
impl<T> CachePadded<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }
}
impl<T> std::ops::Deref for CachePadded<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

// A missing or misspelled cfg must not silently produce an unpadded build.
const _: () = assert!(
    align_of::<CachePadded<std::sync::atomic::AtomicUsize>>() >= 64,
    "CachePadded lost its alignment: check the channel_pad cfg arms"
);
