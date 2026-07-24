use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use tokio::sync::{mpsc, Notify};
use tokio_util::sync::CancellationToken;

use super::scheduler::{
    LocalModelScheduler, SchedulerError, SchedulerJobKey, SchedulerLifecycleState,
};
use super::types::AdviserId;

fn key(run: &str, adviser: AdviserId) -> SchedulerJobKey {
    SchedulerJobKey::new(run, adviser).expect("valid scheduler key")
}

#[tokio::test(flavor = "current_thread")]
async fn capacity_one_is_fifo_and_emits_lifecycle_changes() {
    let scheduler = LocalModelScheduler::new(1).expect("capacity one");
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let first_release = Arc::new(Notify::new());

    let first_scheduler = scheduler.clone();
    let first_release_task = Arc::clone(&first_release);
    let first = tokio::spawn(async move {
        first_scheduler
            .schedule(
                key("run-fifo", AdviserId::Operations),
                CancellationToken::new(),
                move |_| async move {
                    started_tx.send(AdviserId::Operations).ok();
                    first_release_task.notified().await;
                    Ok::<_, &'static str>("operations")
                },
            )
            .await
    });
    assert_eq!(started_rx.recv().await, Some(AdviserId::Operations));

    let second_scheduler = scheduler.clone();
    let (second_tx, mut second_rx) = mpsc::unbounded_channel();
    let second = tokio::spawn(async move {
        second_scheduler
            .schedule(
                key("run-fifo", AdviserId::Navigation),
                CancellationToken::new(),
                move |_| async move {
                    second_tx.send(AdviserId::Navigation).ok();
                    Ok::<_, &'static str>("navigation")
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(second_rx.try_recv().is_err());
    first_release.notify_one();

    assert_eq!(
        first.await.expect("first join").expect("first success"),
        "operations"
    );
    assert_eq!(
        second.await.expect("second join").expect("second success"),
        "navigation"
    );
    assert_eq!(second_rx.recv().await, Some(AdviserId::Navigation));

    let states = scheduler
        .lifecycle_history()
        .into_iter()
        .map(|event| (event.key().adviser(), event.state()))
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![
            (AdviserId::Operations, SchedulerLifecycleState::Queued),
            (AdviserId::Operations, SchedulerLifecycleState::Running),
            (AdviserId::Navigation, SchedulerLifecycleState::Queued),
            (AdviserId::Operations, SchedulerLifecycleState::Completed),
            (AdviserId::Navigation, SchedulerLifecycleState::Running),
            (AdviserId::Navigation, SchedulerLifecycleState::Completed),
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn capacity_two_is_bounded_and_results_remain_input_order() {
    let scheduler = LocalModelScheduler::new(2).expect("capacity two");
    let release = Arc::new(Notify::new());
    let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let maximum = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let jobs = [
        (AdviserId::Operations, 30_u64),
        (AdviserId::Navigation, 5),
        (AdviserId::DailyRoutine, 1),
    ]
    .into_iter()
    .map(|(adviser, delay)| {
        let scheduler = scheduler.clone();
        let release = Arc::clone(&release);
        let running = Arc::clone(&running);
        let maximum = Arc::clone(&maximum);
        async move {
            scheduler
                .schedule(
                    key("run-two", adviser),
                    CancellationToken::new(),
                    move |_| async move {
                        let now = running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        maximum.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                        release.notified().await;
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        Ok::<_, &'static str>(adviser)
                    },
                )
                .await
        }
    })
    .collect::<Vec<_>>();

    let joined = tokio::spawn(async move { join_all(jobs).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(maximum.load(std::sync::atomic::Ordering::SeqCst), 2);
    release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(20)).await;
    release.notify_waiters();

    let results = joined
        .await
        .expect("batch join")
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("batch success");
    assert_eq!(
        results,
        vec![
            AdviserId::Operations,
            AdviserId::Navigation,
            AdviserId::DailyRoutine,
        ]
    );
    assert_eq!(maximum.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(LocalModelScheduler::new(0).is_err());
    assert!(LocalModelScheduler::new(3).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn duplicate_keys_are_rejected_while_first_job_is_active() {
    let scheduler = LocalModelScheduler::new(1).expect("scheduler");
    let release = Arc::new(Notify::new());
    let first_scheduler = scheduler.clone();
    let first_release = Arc::clone(&release);
    let first = tokio::spawn(async move {
        first_scheduler
            .schedule(
                key("duplicate", AdviserId::Operations),
                CancellationToken::new(),
                move |_| async move {
                    first_release.notified().await;
                    Ok::<_, &'static str>(())
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;

    let duplicate = scheduler
        .schedule(
            key("duplicate", AdviserId::Operations),
            CancellationToken::new(),
            |_| async { Ok::<_, &'static str>(()) },
        )
        .await;
    assert_eq!(duplicate, Err(SchedulerError::Duplicate));
    release.notify_one();
    assert!(first.await.expect("first join").is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn queued_and_running_cancellation_are_isolated() {
    let scheduler = LocalModelScheduler::new(1).expect("scheduler");
    let blocker_release = Arc::new(Notify::new());
    let blocker_scheduler = scheduler.clone();
    let blocker_release_task = Arc::clone(&blocker_release);
    let blocker = tokio::spawn(async move {
        blocker_scheduler
            .schedule(
                key("cancel", AdviserId::Operations),
                CancellationToken::new(),
                move |_| async move {
                    blocker_release_task.notified().await;
                    Ok::<_, &'static str>(())
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;

    let queued_token = CancellationToken::new();
    let queued_scheduler = scheduler.clone();
    let queued_child = queued_token.clone();
    let queued = tokio::spawn(async move {
        queued_scheduler
            .schedule(
                key("cancel", AdviserId::Navigation),
                queued_child,
                |_| async { Ok::<_, &'static str>(()) },
            )
            .await
    });
    queued_token.cancel();
    assert_eq!(
        queued.await.expect("queued join"),
        Err(SchedulerError::Cancelled)
    );

    blocker_release.notify_one();
    assert!(blocker.await.expect("blocker join").is_ok());

    let running_token = CancellationToken::new();
    let running_child = running_token.clone();
    let running_scheduler = scheduler.clone();
    let running = tokio::spawn(async move {
        running_scheduler
            .schedule(
                key("cancel-running", AdviserId::Plans),
                running_child,
                |cancellation| async move {
                    cancellation.cancelled().await;
                    Err::<(), _>("cancelled")
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    running_token.cancel();
    assert_eq!(
        running.await.expect("running join"),
        Err(SchedulerError::Cancelled)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn abort_ignoring_work_retains_capacity_until_it_settles() {
    let scheduler = LocalModelScheduler::new(1).expect("scheduler");
    let release = Arc::new(Notify::new());
    let token = CancellationToken::new();
    let first_scheduler = scheduler.clone();
    let first_release = Arc::clone(&release);
    let first_child = token.clone();
    let first = tokio::spawn(async move {
        first_scheduler
            .schedule(
                key("ignore", AdviserId::Operations),
                first_child,
                move |_| async move {
                    first_release.notified().await;
                    Ok::<_, &'static str>(())
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    token.cancel();

    let (second_started_tx, mut second_started_rx) = mpsc::unbounded_channel();
    let second_scheduler = scheduler.clone();
    let second = tokio::spawn(async move {
        second_scheduler
            .schedule(
                key("ignore", AdviserId::Navigation),
                CancellationToken::new(),
                move |_| async move {
                    second_started_tx.send(()).ok();
                    Ok::<_, &'static str>(())
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(second_started_rx.try_recv().is_err());

    release.notify_one();
    assert_eq!(
        first.await.expect("first join"),
        Err(SchedulerError::Cancelled)
    );
    assert!(second.await.expect("second join").is_ok());
    assert_eq!(second_started_rx.recv().await, Some(()));
}

#[tokio::test(flavor = "current_thread")]
async fn panic_and_task_error_do_not_poison_later_jobs() {
    let scheduler = LocalModelScheduler::new(1).expect("scheduler");
    let panicked = scheduler
        .schedule(
            key("isolation", AdviserId::Operations),
            CancellationToken::new(),
            |_| async move {
                panic!("model-visible secret must not escape");
                #[allow(unreachable_code)]
                Ok::<(), &'static str>(())
            },
        )
        .await;
    assert_eq!(panicked, Err(SchedulerError::Panicked));

    let failed = scheduler
        .schedule(
            key("isolation", AdviserId::Navigation),
            CancellationToken::new(),
            |_| async { Err::<(), _>("transport") },
        )
        .await;
    assert_eq!(failed, Err(SchedulerError::Task("transport")));

    let healthy = scheduler
        .schedule(
            key("isolation", AdviserId::DailyRoutine),
            CancellationToken::new(),
            |_| async { Ok::<_, &'static str>("healthy") },
        )
        .await;
    assert_eq!(healthy, Ok("healthy"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_key_remains_active_until_its_terminal_lifecycle_event_is_visible() {
    for round in 0..256 {
        let scheduler = LocalModelScheduler::new(1).expect("scheduler");
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let first_scheduler = scheduler.clone();
        let first_barrier = Arc::clone(&barrier);
        let first = tokio::spawn(async move {
            first_scheduler
                .schedule(
                    key("terminal-order", AdviserId::Operations),
                    CancellationToken::new(),
                    move |_| async move {
                        first_barrier.wait().await;
                        Ok::<_, &'static str>(())
                    },
                )
                .await
        });
        let (active, running_history) = loop {
            let snapshot = scheduler.state_snapshot();
            if snapshot.0 == 1
                && snapshot.1.last().map(|event| event.state())
                    == Some(SchedulerLifecycleState::Running)
            {
                break snapshot;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(active, 1);
        assert_eq!(
            running_history.last().map(|event| event.state()),
            Some(SchedulerLifecycleState::Running)
        );

        let second_scheduler = scheduler.clone();
        let second_barrier = Arc::clone(&barrier);
        let second = tokio::spawn(async move {
            second_barrier.wait().await;
            loop {
                match second_scheduler
                    .schedule(
                        key("terminal-order", AdviserId::Operations),
                        CancellationToken::new(),
                        |_| async { Ok::<_, &'static str>(()) },
                    )
                    .await
                {
                    Err(SchedulerError::Duplicate) => tokio::task::yield_now().await,
                    result => break result,
                }
            }
        });

        assert!(first.await.expect("first join").is_ok(), "round {round}");
        assert!(second.await.expect("second join").is_ok(), "round {round}");
        let (active, history) = scheduler.state_snapshot();
        assert_eq!(active, 0);
        let states = history
            .into_iter()
            .map(|event| event.state())
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                SchedulerLifecycleState::Queued,
                SchedulerLifecycleState::Running,
                SchedulerLifecycleState::Completed,
                SchedulerLifecycleState::Queued,
                SchedulerLifecycleState::Running,
                SchedulerLifecycleState::Completed,
            ],
            "round {round}"
        );
    }
}
