use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[test]
fn startup_failure_is_returned_after_worker_exit() {
    let (tx, rx) = mpsc::sync_channel(1);
    let exited = Arc::new(AtomicBool::new(false));
    let exited_worker = Arc::clone(&exited);
    let handle = std::thread::spawn(move || {
        tx.send(Err("output unavailable".to_string()))
            .expect("startup receiver");
        exited_worker.store(true, Ordering::Release);
    });

    assert_eq!(
        await_worker_startup(handle, rx).expect_err("startup must fail"),
        "output unavailable"
    );
    assert!(exited.load(Ordering::Acquire));
}

#[test]
fn worker_exit_before_readiness_is_a_startup_error() {
    let (tx, rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let handle = std::thread::spawn(move || drop(tx));

    assert!(await_worker_startup(handle, rx)
        .expect_err("closed startup channel must fail")
        .contains("before reporting readiness"));
}

#[test]
fn ready_ack_precedes_pipeline_publication_boundary() {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        tx.send(Ok(())).expect("startup receiver");
    });

    let handle = await_worker_startup(handle, rx).expect("ready worker");
    handle.join().expect("worker exits");
}
