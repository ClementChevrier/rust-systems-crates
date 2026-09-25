#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(loom)]
mod loom_tests {
    use loom::thread;

    use crate::snapshot::channel;

    // First test is hand made the rest is done by IA!
    #[test]
    fn life_cycle() {
        loom::model(|| {
            let vec_ini = vec![0; 1];
            let (mut reader, mut writer) = channel(vec_ini);

            let t_reader = thread::spawn(move || {
                assert_eq!(reader.current(), &vec![0; 1]);
                // Wait for writer to publish
                while let Ok(is_update) = reader.update()
                    && !is_update
                {
                    thread::yield_now()
                }
                assert_eq!(reader.current(), &vec![0; 2]);

                // Wait for writer to close
                while !reader.is_closed() {
                    thread::yield_now()
                }
                assert!(reader.is_closed())
            });

            let t_writer = thread::spawn(move || {
                writer.publish(vec![0; 2]).expect("channel is open");
                writer.close();
                assert!(writer.is_closed())
            });

            t_writer.join().unwrap();
            t_reader.join().unwrap();
        });
    }

    #[test]
    fn publish_is_visible() {
        loom::model(|| {
            let (mut reader, mut writer) = channel(vec![0]);

            let t_writer = thread::spawn(move || {
                assert!(writer.publish(vec![0, 1]).is_ok());
            });

            let t_reader = thread::spawn(move || {
                loop {
                    match reader.update() {
                        Ok(true) => {
                            assert_eq!(reader.current(), &vec![0, 1]);
                            break;
                        }
                        Ok(false) => thread::yield_now(),
                        Err(_) => panic!("unexpected disconnect"),
                    }
                }
            });

            t_writer.join().unwrap();
            t_reader.join().unwrap();
        });
    }

    #[test]
    fn writer_close_is_observed() {
        loom::model(|| {
            let (reader, mut writer) = channel(0);

            let t_writer = thread::spawn(move || {
                writer.close();
            });

            let t_reader = thread::spawn(move || {
                loop {
                    if reader.is_closed() {
                        break;
                    }

                    thread::yield_now();
                }
            });

            t_writer.join().unwrap();
            t_reader.join().unwrap();
        });
    }

    #[test]
    fn publish_then_close() {
        loom::model(|| {
            let (mut reader, mut writer) = channel(0);

            let t_writer = thread::spawn(move || {
                writer.publish(1).expect("channel is open");
                thread::yield_now();
                writer.close();
            });

            let t_reader = thread::spawn(move || {
                while !reader.update().unwrap() {
                    thread::yield_now();
                }

                assert_eq!(*reader.current(), 1);

                while !reader.is_closed() {
                    thread::yield_now();
                }
            });

            t_writer.join().unwrap();
            t_reader.join().unwrap();
        });
    }

    #[test]
    fn reader_close_races_with_publish() {
        loom::model(|| {
            let (mut reader, mut writer) = channel(0);

            let t_reader = thread::spawn(move || {
                reader.close();
            });

            let t_writer = thread::spawn(move || {
                let _ = writer.publish(1);
            });

            t_reader.join().unwrap();
            t_writer.join().unwrap();
        });
    }

    #[test]
    fn update_races_with_writer_close() {
        loom::model(|| {
            let (mut reader, mut writer) = channel(0);

            let t_writer = thread::spawn(move || {
                writer.close();
            });

            let t_reader = thread::spawn(move || {
                loop {
                    match reader.update() {
                        Ok(false) => {
                            if reader.is_closed() {
                                break;
                            }
                            thread::yield_now();
                        }
                        Ok(true) => panic!("no snapshot should exist"),
                        Err(_) => break,
                    }
                }
            });

            t_writer.join().unwrap();
            t_reader.join().unwrap();
        });
    }

    #[test]
    fn reader_never_moves_backwards_and_ends_on_the_newest() {
        loom::model(|| {
            let (mut reader, mut writer) = channel(0u32);

            let t_writer = thread::spawn(move || {
                let _ = writer.publish(1);
                let _ = writer.publish(2);
            });

            let t_reader = thread::spawn(move || {
                let mut seen = *reader.current();
                loop {
                    match reader.update() {
                        Ok(_) => {
                            let now = *reader.current();
                            assert!(now >= seen, "a snapshot must never move backwards");
                            seen = now;
                            if seen == 2 {
                                break;
                            }
                        }
                        // the writer dropped: we are on the tail, nothing more comes
                        Err(_) => break,
                    }
                    thread::yield_now();
                }
                seen
            });

            t_writer.join().unwrap();
            assert_eq!(t_reader.join().unwrap(), 2);
        });
    }

    #[test]
    fn dropping_writer_closes_reader() {
        loom::model(|| {
            let (reader, writer) = channel(0);

            let t_writer = thread::spawn(move || {
                drop(writer);
            });

            let t_reader = thread::spawn(move || {
                loop {
                    if reader.is_closed() {
                        break;
                    }

                    thread::yield_now();
                }
            });

            t_writer.join().unwrap();
            t_reader.join().unwrap();
        });
    }

    #[test]
    fn close_is_idempotent() {
        loom::model(|| {
            let (mut reader, mut writer) = channel(0);

            reader.close();
            reader.close();

            writer.close();
            writer.close();

            assert!(reader.is_closed());
            assert!(writer.is_closed());
        });
    }
}
