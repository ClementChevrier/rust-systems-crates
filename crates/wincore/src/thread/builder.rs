use std::{sync::mpsc::sync_channel, thread};

use super::{
    affinity::ProcessorId,
    control::*,
    outcome::*,
    priority::{BoostPolicy, ThreadPriority},
};

/// `std::thread::Builder` extended with CPU pinning, Win32 priority and boost control.
///
/// Settings are applied from inside the new thread, before its closure runs. Each
/// one is optional: left unset, it is not touched and the thread keeps the Windows
/// default.
#[derive(Debug)]
pub struct Builder {
    name: Option<String>,
    prio: Option<ThreadPriority>,
    boost: Option<BoostPolicy>,
    cpu: Option<ProcessorId>,
    stack_size: Option<usize>,
}
impl Builder {
    /// An empty builder: nothing requested, behaves exactly like `std`.
    pub fn new() -> Self {
        Self {
            name: None,
            prio: None,
            boost: None,
            cpu: None,
            stack_size: None,
        }
    }

    /// Stack size of the new thread, in bytes. Passed straight through to `std`.
    pub fn stack_size(mut self, size: usize) -> Self {
        self.stack_size = Some(size);
        self
    }

    /// Thread priority level. Combines with the process class to give the base
    /// priority — see the table in [`super::priority`].
    pub fn priority(mut self, prio: ThreadPriority) -> Self {
        self.prio = Some(prio);
        self
    }

    /// Whether Windows may temporarily raise this thread's priority. Disable it on
    /// any thread whose latency has to stay predictable.
    pub fn boost_policy(mut self, policy: BoostPolicy) -> Self {
        self.boost = Some(policy);
        self
    }

    /// Pins the thread to a single logical CPU through the Win32 group affinity.
    ///
    /// Fails if `cpu.number` exceeds the width of the affinity mask (64 per group);
    /// the failure surfaces in [`ConfigReport::pin`] rather than being raised here.
    pub fn pin_to(mut self, cpu: ProcessorId) -> Self {
        self.cpu = Some(cpu);
        self
    }

    /// Thread name, readable through `std::thread::current().name()`.
    ///
    /// The name stays on the Rust side: Windows never sees it, so it shows up
    /// neither in Process Explorer nor in ETW traces.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Starts the thread and waits until it has applied its configuration.
    ///
    /// Unlike `std::thread::Builder::spawn`, blocks until the new thread is done
    /// configuring itself, so the caller cannot observe a partially set up thread.
    ///
    /// Returns [`Spawned`] even when settings failed — the detail is in
    /// [`ConfigReport`]. Only a thread that could not be created, or that died
    /// before reporting back, yields a [`SpawnError`].
    pub fn spawn<F>(self, f: F) -> Result<Spawned, SpawnError>
    where
        F: FnOnce() + Send + 'static,
    {
        // Carries the configuration report back from the new thread.
        let (init_tx, init_rx) = sync_channel::<ConfigReport>(1);

        let mut builder = thread::Builder::new();
        if let Some(name) = self.name {
            builder = builder.name(name);
        }

        if let Some(size) = self.stack_size {
            builder = builder.stack_size(size);
        }
        let handle = builder.spawn(move || {
            // The receiver waits on this message below, and the channel has
            // room for it, so the send cannot fail in practice. If it ever did,
            // `recv` reports `InitFailed` and the thread still runs.
            let _ = init_tx.send(configure_current_thread(self.prio, self.cpu, self.boost));
            f()
        })?;

        match init_rx.recv() {
            Ok(config) => Ok(Spawned { handle, config }),
            // Only fails if the new thread died before reporting.
            Err(_) => Err(SpawnError::InitFailed(handle)),
        }
    }
}
impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

fn configure_current_thread(
    prio: Option<ThreadPriority>,
    cpu: Option<ProcessorId>,
    boost: Option<BoostPolicy>,
) -> ConfigReport {
    ConfigReport {
        pin: Setting::apply(cpu, pin_current_thread_to_cpu),
        prio: Setting::apply(prio, set_current_priority),
        boost: Setting::apply(boost, set_current_boost_policy),
    }
}

#[cfg(test)]
#[path = "tests/builder.rs"]
mod builder_test;
