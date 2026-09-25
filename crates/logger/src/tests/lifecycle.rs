#![allow(clippy::unwrap_used, clippy::panic)]

//! A single test, on purpose: the sender (`OnceLock`) and `MIN_LEVEL` are
//! process-global, and `cargo test` runs in parallel. Every phase that touches
//! the singleton or the level must therefore be sequenced here; the other
//! facets have hermetic tests (worker.rs, builder.rs, file_manager.rs).

use std::fs;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use crate::macros::{log_debug, log_info};
use crate::{Error, LogLevel, Logger};

const POLL: Duration = Duration::from_millis(25);
const FLUSH: Duration = Duration::from_millis(50);
const WAIT: Duration = Duration::from_secs(5);

fn read_all_logs(dir: &Path) -> String {
    let mut out = String::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => out.push_str(&read_all_logs(&path)),
            Ok(_) => {
                if let Ok(content) = fs::read_to_string(&path) {
                    out.push_str(&content);
                }
            }
            Err(_) => continue,
        }
    }
    out
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

#[test]
#[cfg_attr(miri, ignore = "end to end: disk I/O, polling thread and real timings")]
fn full_lifecycle() {
    let dir = std::env::temp_dir().join(format!("logger_lifecycle_{}", std::process::id()));
    // Outside the log folder, so read_all_logs does not pick it up.
    let level_file =
        std::env::temp_dir().join(format!("logger_lifecycle_{}.ctl", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&level_file);

    // --- Before init: routed to stderr, no panic, nothing created.
    crate::__private::log(LogLevel::Error, "PRE", format_args!("before init"));
    assert!(!dir.exists());

    let (tx, rx) = channel();
    let mut logger = Logger::builder()
        .output(&dir, "log")
        .buffer_len(8192)
        .sync_policy(crate::SyncPolicy::OnLevel(LogLevel::Error))
        .min_level(LogLevel::Info)
        .runtime_level_file(&level_file)
        .poll_interval(POLL)
        .thread_init(move || {
            let _ = tx.send(std::thread::current().name().map(String::from));
        })
        .build()
        .expect("first init must succeed");

    // --- Startup hook, on the named thread.
    let hook_thread = rx.recv_timeout(WAIT).expect("hook must run");
    assert_eq!(hook_thread.as_deref(), Some("Thread-Logger"));

    // --- Explicit level: applied AND persisted to the control file.
    assert_eq!(
        fs::read_to_string(&level_file).expect("level file must be seeded"),
        "INFO"
    );
    assert!(crate::__private::allowed(LogLevel::Error));
    assert!(crate::__private::allowed(LogLevel::Info));
    assert!(!crate::__private::allowed(LogLevel::Debug));

    // --- Filtrage.
    log_debug!("TEST", "must be filtered out");
    log_info!("TEST", "hello {}", 42);

    // --- Singleton: a second build is refused without any side effect.
    let default_dir = Path::new(".").join(".logs");
    let default_dir_existed = default_dir.exists();
    assert!(matches!(
        Logger::builder().min_level(LogLevel::Critical).build(),
        Err(Error::AlreadyInitialized)
    ));
    assert_eq!(
        default_dir.exists(),
        default_dir_existed,
        "a refused build must not create files"
    );
    assert!(
        crate::__private::allowed(LogLevel::Info),
        "a refused build must not change the level"
    );

    // --- Live level change through the control file.
    fs::write(&level_file, "debug").expect("write level file");
    assert!(
        wait_until(WAIT, || crate::__private::allowed(LogLevel::Debug)),
        "runtime level change must be observed within the poll interval"
    );
    log_debug!("TEST", "debug now visible");

    // --- Invalid content: level kept, no panic.
    fs::write(&level_file, "not_a_level").expect("write level file");
    std::thread::sleep(POLL * 4);
    assert!(crate::__private::allowed(LogLevel::Debug));

    // --- File deleted while running: level kept, no panic.
    fs::remove_file(&level_file).expect("remove level file");
    std::thread::sleep(POLL * 4);
    assert!(crate::__private::allowed(LogLevel::Debug));

    // --- Flush: the line reaches the disk without a shutdown.
    for i in 0..2000 {
        log_info!("TEST", "hello {i}");
    }
    assert!(wait_until(WAIT, || read_all_logs(&dir).contains("hello 42")));

    // --- Idempotent shutdown (Drop makes an implicit third call).
    logger.shutdown();
    logger.shutdown();

    let content = read_all_logs(&dir);
    assert!(content.contains("Started!"), "content: {content}");
    assert!(content.contains("hello 42"));
    assert!(content.contains("debug now visible"));
    assert!(!content.contains("must be filtered out"));
    assert_eq!(content.matches("Received shutdown command!").count(), 1);

    // --- Logging after shutdown: lost without a panic, files unchanged.
    log_info!("TEST", "after shutdown");
    std::thread::sleep(FLUSH * 2);
    assert!(!read_all_logs(&dir).contains("after shutdown"));

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&level_file);
}
