#![allow(clippy::unwrap_used, clippy::panic)]

//! The worker is tested directly, without the singleton: each test owns its
//! channel, its FileManager and its thread, so the tests are deterministic and
//! can run in parallel. None of them touches MIN_LEVEL (no level file): the
//! live level change is covered, sequenced, by lifecycle.rs.

use super::LogWorker;
use crate::config::{
    builder::{DEFAULT_POLL_LEVEL_INTERVAL, DEFAULT_SLEEP_MAIN_LOOP},
    logger::CHANNEL_LEN,
};
use crate::file_manager::FileManager;
use crate::message::{LogCommand, LogLevel, LogMessage};

use channel::mpsc;

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::SystemTime;

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("logger_worker_{}_{tag}", std::process::id()))
}

fn msg(body: &str) -> LogCommand {
    LogCommand::Message(LogMessage {
        level: LogLevel::Info,
        ts: SystemTime::now(),
        title: "TEST",
        body: body.into(),
    })
}

fn spawn_worker(
    dir: &Path,
) -> (
    mpsc::Producer<LogCommand, CHANNEL_LEN>,
    thread::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::channel();
    (sender, spawn_worker_on(dir, receiver))
}

/// Starts a worker on an existing channel, which the test may have filled first.
fn spawn_worker_on(
    dir: &Path,
    receiver: mpsc::Consumer<LogCommand, CHANNEL_LEN>,
) -> thread::JoinHandle<()> {
    let manager = FileManager::new(
        dir.to_path_buf(),
        "log".to_string(),
        u64::MAX,
        8192,
        false,
        crate::SyncPolicy::OnRotation,
    )
    .expect("file manager must be creatable");
    thread::spawn(move || {
        let mut worker = LogWorker::new(
            manager,
            DEFAULT_SLEEP_MAIN_LOOP,
            receiver,
            None,
            DEFAULT_POLL_LEVEL_INTERVAL,
        );
        worker.run();
    })
}

fn log_content(dir: &Path) -> String {
    fs::read_to_string(dir.join("log_0.log")).unwrap_or_default()
}

#[test]
fn worker_writes_markers_and_messages_in_order() {
    let dir = temp_dir("order");
    let _ = fs::remove_dir_all(&dir);
    let (sender, handle) = spawn_worker(&dir);

    sender.push(msg("first message")).expect("send");
    sender.push(msg("second message")).expect("send");
    sender.push(LogCommand::Shutdown).expect("send");
    handle.join().expect("worker must not panic");

    // NEVER_FLUSH: if these lines reached the disk, the shutdown flush wrote them.
    let content = log_content(&dir);
    let started = content.find("Started!").expect("start marker");
    let first = content.find("first message").expect("first message");
    let second = content.find("second message").expect("second message");
    let stopped = content
        .find("Received shutdown command!")
        .expect("shutdown marker");
    assert!(
        started < first && first < second && second < stopped,
        "content: {content}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dropping_all_senders_shuts_the_worker_down() {
    let dir = temp_dir("disconnect");
    let _ = fs::remove_dir_all(&dir);
    let (sender, handle) = spawn_worker(&dir);

    sender.push(msg("last words")).expect("send");
    drop(sender);
    handle
        .join()
        .expect("worker must stop cleanly on disconnect");

    let content = log_content(&dir);
    assert!(content.contains("last words"));
    assert!(content.contains("All senders are disconnected. Closing Logger!"));

    let _ = fs::remove_dir_all(&dir);
}

// `push` fails fast: retry on ChannelFull so the test loses nothing.
fn push_blocking(sender: &mpsc::Producer<LogCommand, CHANNEL_LEN>, mut cmd: LogCommand) {
    loop {
        match sender.push(cmd) {
            Ok(()) => return,
            Err(channel::Error::ChannelFull(returned)) => {
                cmd = returned;
                std::hint::spin_loop();
            }
            Err(channel::Error::ReceiverDisconnected(_)) => panic!("worker disconnected mid-burst"),
        }
    }
}

#[test]
fn a_burst_larger_than_the_channel_is_fully_written_in_order() {
    const BURST: usize = CHANNEL_LEN * 4; // > capacity: push_blocking retries on ChannelFull, nothing is lost
    let dir = temp_dir("burst");
    let _ = fs::remove_dir_all(&dir);
    let (sender, handle) = spawn_worker(&dir);

    for i in 0..BURST {
        push_blocking(&sender, msg(&format!("burst {i:04}")));
    }
    push_blocking(&sender, LogCommand::Shutdown);
    handle.join().expect("worker must not panic");

    let content = log_content(&dir);
    let mut previous = 0;
    for i in 0..BURST {
        let position = content
            .find(&format!("burst {i:04}"))
            .unwrap_or_else(|| panic!("burst {i:04} missing"));
        assert!(position >= previous, "burst {i:04} out of order");
        previous = position;
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Regression: `Logger::shutdown` pushes `Shutdown` then closes the channel at
/// once. With a full channel, the command can only get in while the worker is
/// emptying an older snapshot; the worker then saw `closed`, took the "senders
/// dead" path and silently discarded the command, so the shutdown marker was
/// never written. Filling the channel before the worker starts makes that
/// interleaving the normal case.
#[test]
fn shutdown_pushed_into_a_full_channel_is_honoured_despite_the_close() {
    let dir = temp_dir("full_shutdown");
    let _ = fs::remove_dir_all(&dir);
    let (sender, receiver) = mpsc::channel::<LogCommand, CHANNEL_LEN>();
    for i in 0..CHANNEL_LEN {
        push_blocking(&sender, msg(&format!("queued {i:04}")));
    }
    let handle = spawn_worker_on(&dir, receiver);

    push_blocking(&sender, LogCommand::Shutdown);
    sender.close();
    handle.join().expect("worker must not panic");

    let content = log_content(&dir);
    assert!(content.contains(&format!("queued {:04}", CHANNEL_LEN - 1)));
    assert_eq!(
        content.matches("Received shutdown command!").count(),
        1,
        "the shutdown marker must be written"
    );
    assert!(!content.contains("All senders are disconnected"));
    let _ = fs::remove_dir_all(&dir);
}
