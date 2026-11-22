use crate::{TestScheduler, TestSchedulerConfig};
use futures::future;
use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering::SeqCst},
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

// A timed block on the test thread used to lock `rng` and then `state`, while scheduling from
// another thread locks `state` and then `rng`, so the two could deadlock.
// https://github.com/zed-industries/zed/issues/66801
#[test]
#[cfg_attr(miri, ignore)]
fn test_block_with_timeout_does_not_deadlock_with_off_thread_scheduling() {
    const SPAWN_COUNT: usize = 2_000;

    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let scheduler = Arc::new(TestScheduler::new(TestSchedulerConfig {
            allow_parking: true,
            timeout_ticks: 0..=0,
            ..Default::default()
        }));
        let foreground = scheduler.foreground();
        let background = scheduler.background();

        let start = Arc::new(Barrier::new(2));
        let spawner_finished = Arc::new(AtomicBool::new(false));
        let spawner = thread::spawn({
            let start = start.clone();
            let spawner_finished = spawner_finished.clone();
            move || {
                start.wait();
                for _ in 0..SPAWN_COUNT {
                    background.spawn(async {}).detach();
                }
                spawner_finished.store(true, SeqCst);
            }
        });

        start.wait();
        while !spawner_finished.load(SeqCst) {
            let timed_out = foreground
                .block_with_timeout(Duration::ZERO, future::pending::<()>())
                .is_err();
            assert!(timed_out);
        }
        spawner.join().expect("spawner thread panicked");
        scheduler.run();
        done_tx.send(()).ok();
    });

    // A deadlock fails the test here instead of hanging it.
    match done_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(()) => {}
        Err(RecvTimeoutError::Timeout) => {
            panic!("timed block on the test thread deadlocked with scheduling from another thread")
        }
        Err(RecvTimeoutError::Disconnected) => panic!("test scheduler thread panicked"),
    }
}
