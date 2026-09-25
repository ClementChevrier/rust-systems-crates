#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(not(loom))]
mod std_tests {
    use crate::snapshot::channel;

    #[test]
    fn reader_catches_up_after_writer_dropped() {
        let (mut reader, mut writer) = channel(0u32);

        writer.publish(1).expect("channel is open");
        writer.publish(2).expect("channel is open");
        drop(writer);

        assert!(
            reader
                .update()
                .expect("pending snapshots drain before the close")
        );
        assert_eq!(*reader.current(), 2);

        assert!(reader.update().is_err(), "closed once the tail is reached");
    }

    #[test]
    fn update_is_latest_wins_not_a_queue() {
        let (mut reader, mut writer) = channel(0u32);

        writer.publish(1).expect("channel is open");
        writer.publish(2).expect("channel is open");
        writer.publish(3).expect("channel is open");

        assert!(reader.update().expect("one call reaches the tail"));
        assert_eq!(*reader.current(), 3, "intermediates are skipped and freed");
    }

    /// Counts its drops, to check that every published value is freed once.
    struct Counted(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn every_value_is_freed_once(writer_first: bool) {
        use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};

        let drops = Arc::new(AtomicUsize::new(0));
        let (mut reader, mut writer) = channel(Counted(Arc::clone(&drops)));
        for _ in 0..3 {
            assert!(writer.publish(Counted(Arc::clone(&drops))).is_ok());
        }
        assert!(reader.update().expect("channel is open"));
        assert!(writer.publish(Counted(Arc::clone(&drops))).is_ok());

        if writer_first {
            drop(writer);
            drop(reader);
        } else {
            drop(reader);
            drop(writer);
        }
        // The seed plus four publishes, the final node included.
        assert_eq!(drops.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn nothing_leaks_when_the_writer_drops_first() {
        every_value_is_freed_once(true);
    }

    #[test]
    fn nothing_leaks_when_the_reader_drops_first() {
        every_value_is_freed_once(false);
    }
}
