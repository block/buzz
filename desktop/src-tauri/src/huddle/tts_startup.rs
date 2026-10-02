use std::{sync::mpsc, thread};

pub(super) fn await_worker_startup(
    handle: thread::JoinHandle<()>,
    startup_rx: mpsc::Receiver<Result<(), String>>,
) -> Result<thread::JoinHandle<()>, String> {
    match startup_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(error)) => {
            let _ = handle.join();
            Err(error)
        }
        Err(error) => {
            let _ = handle.join();
            Err(format!(
                "TTS worker exited before reporting readiness: {error}"
            ))
        }
    }
}

#[cfg(test)]
#[path = "tts_startup_tests.rs"]
mod tests;
