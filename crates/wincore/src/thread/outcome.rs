use std::{io, thread::JoinHandle};

use super::{
    affinity::ProcessorId,
    priority::{BoostPolicy, ThreadPriority},
};

/// Outcome of one setting requested at spawn time.
///
/// The requested value is kept on success and on failure alike: a log line then
/// stands on its own, and the registry can compare intent against what Windows
/// actually reports.
#[derive(Debug)]
pub enum Setting<T> {
    /// The caller did not request this setting.
    Skipped,
    /// Applied, with the requested value.
    Applied(T),
    /// Windows refused it; the thread still runs, with its default value.
    Failed {
        /// The value that was asked for.
        wanted: T,
        /// The Windows error.
        source: io::Error,
    },
}

impl<T> Setting<T> {
    pub(super) fn apply(wanted: Option<T>, action: impl FnOnce(T) -> Result<(), io::Error>) -> Self
    where
        T: Copy,
    {
        let Some(wanted) = wanted else {
            return Self::Skipped;
        };
        match action(wanted) {
            Ok(()) => Self::Applied(wanted),
            Err(source) => Self::Failed { wanted, source },
        }
    }

    /// The requested value, whether or not it was applied. `None` if not requested.
    pub fn wanted(&self) -> Option<&T> {
        match self {
            Self::Skipped => None,
            Self::Applied(wanted) | Self::Failed { wanted, .. } => Some(wanted),
        }
    }

    /// The Windows error, if the setting was refused.
    pub fn failure(&self) -> Option<&io::Error> {
        match self {
            Self::Failed { source, .. } => Some(source),
            _ => None,
        }
    }
}
impl<T: std::fmt::Display> std::fmt::Display for Setting<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "skipped"),
            Self::Applied(wanted) => write!(f, "{wanted}"),
            Self::Failed { wanted, source } => write!(f, "failed(wanted={wanted} err={source})"),
        }
    }
}

/// Names one setting inside a [`ConfigReport`], for log lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// CPU pinning.
    Pin,
    /// Thread priority level.
    Priority,
    /// Boost policy.
    Boost,
}

impl std::fmt::Display for SettingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pin => "pin",
            Self::Priority => "prio",
            Self::Boost => "boost",
        })
    }
}

/// What the thread ended up with, setting by setting.
#[derive(Debug)]
pub struct ConfigReport {
    /// CPU pinning.
    pub pin: Setting<ProcessorId>,
    /// Thread priority level.
    pub prio: Setting<ThreadPriority>,
    /// Boost policy.
    pub boost: Setting<BoostPolicy>,
}

impl ConfigReport {
    /// True when no requested setting was refused.
    pub fn is_clean(&self) -> bool {
        self.failures().next().is_none()
    }

    /// The refused settings only, with their Windows error.
    pub fn failures(&self) -> impl Iterator<Item = (SettingKind, &io::Error)> {
        [
            (SettingKind::Pin, self.pin.failure()),
            (SettingKind::Priority, self.prio.failure()),
            (SettingKind::Boost, self.boost.failure()),
        ]
        .into_iter()
        .filter_map(|(kind, source)| source.map(|source| (kind, source)))
    }
}

impl std::fmt::Display for ConfigReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "pin={} prio={} boost={}",
            self.pin, self.prio, self.boost
        )
    }
}

/// A started thread. It runs either way; `config` says in what state.
#[derive(Debug)]
pub struct Spawned {
    /// The thread's handle, as `std::thread::Builder::spawn` returns it.
    pub handle: JoinHandle<()>,
    /// Which of the requested settings took effect.
    pub config: ConfigReport,
}

/// Why [`Builder::spawn`](super::Builder::spawn) returned no running thread.
#[derive(Debug)]
pub enum SpawnError {
    /// The thread started but died before reporting its configuration.
    ///
    /// The `JoinHandle` is handed back: joining here would block forever if the
    /// closure never returns, so the decision belongs to the caller.
    InitFailed(JoinHandle<()>),
    /// `std::thread::Builder::spawn` failed: no thread was created.
    Spawn(io::Error),
}

impl SpawnError {
    /// Hands back the orphaned thread's `JoinHandle`, if there is one.
    pub fn into_handle(self) -> Option<JoinHandle<()>> {
        match self {
            Self::InitFailed(handle) => Some(handle),
            Self::Spawn(_) => None,
        }
    }
}

impl From<io::Error> for SpawnError {
    fn from(value: io::Error) -> Self {
        Self::Spawn(value)
    }
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InitFailed(_) => {
                f.write_str("thread created but died before reporting its configuration")
            }
            Self::Spawn(source) => write!(f, "failed to spawn the thread: {source}"),
        }
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InitFailed(_) => None,
            Self::Spawn(source) => Some(source),
        }
    }
}
