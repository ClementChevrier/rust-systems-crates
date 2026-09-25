/// The other end of the channel is gone.
///
/// Carries the value that could not be delivered, so a caller that wants to
/// route it elsewhere does not lose it. Defaults to `()` for operations that
/// have nothing to hand back, such as a batch refused before any item moved.
pub struct ReceiverDisconnected<T = ()>(pub T);

impl<T> ReceiverDisconnected<T> {
    /// Wraps the value that could not be delivered.
    pub const fn with_value(value: T) -> Self {
        Self(value)
    }

    /// Returns the value that could not be delivered.
    pub fn into_value(self) -> T {
        self.0
    }

    /// Transforms the carried value, keeping the error.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ReceiverDisconnected<U> {
        ReceiverDisconnected(f(self.0))
    }
}

impl Default for ReceiverDisconnected<()> {
    fn default() -> Self {
        Self(())
    }
}

impl<T> std::fmt::Debug for ReceiverDisconnected<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReceiverDisconnected")
    }
}

impl<T> std::fmt::Display for ReceiverDisconnected<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("receiver disconnected")
    }
}

impl<T> std::error::Error for ReceiverDisconnected<T> {}

/// Why a single-item push was refused. Both variants hand the value back.
pub enum Error<T> {
    /// The consumer is gone: nothing pushed from now on will ever be read.
    ReceiverDisconnected(ReceiverDisconnected<T>),
    /// No free slot right now. The waiting policy belongs to the caller.
    ChannelFull(T),
}

impl<T> Error<T> {
    /// Returns the value that was refused, whatever the reason.
    pub fn into_value(self) -> T {
        match self {
            Self::ChannelFull(t) => t,
            Self::ReceiverDisconnected(disconnected) => disconnected.into_value(),
        }
    }

    /// Builds [`Error::ReceiverDisconnected`] around the refused value.
    pub fn disconnected(value: T) -> Self {
        Self::ReceiverDisconnected(ReceiverDisconnected::with_value(value))
    }
}

impl<T> From<ReceiverDisconnected<T>> for Error<T> {
    fn from(value: ReceiverDisconnected<T>) -> Self {
        Self::ReceiverDisconnected(value)
    }
}

impl<T> std::fmt::Debug for Error<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelFull(_) => f.write_str("ChannelFull"),
            Self::ReceiverDisconnected(_) => f.write_str("ReceiverDisconnected"),
        }
    }
}

impl<T> std::fmt::Display for Error<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelFull(_) => f.write_str("channel full"),
            Self::ReceiverDisconnected(_) => f.write_str("receiver disconnected"),
        }
    }
}

impl<T> std::error::Error for Error<T> {}
