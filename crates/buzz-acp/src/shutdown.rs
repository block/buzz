//! One process-wide stop signal, installed before owning children. Respawn and
//! eager/lazy startup share this authority; no second lifecycle/run identity.
use std::{sync::OnceLock, time::Duration};
use tokio::sync::watch;
static SHUTDOWN: OnceLock<watch::Sender<bool>> = OnceLock::new();

pub(crate) fn install() -> std::io::Result<(watch::Sender<bool>, watch::Receiver<bool>)> {
    let (tx, rx) = watch::channel(false);
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let tx = tx.clone();
        tokio::spawn(async move {
            tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} }
            let _ = tx.send(true);
        });
    }
    #[cfg(not(unix))]
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = tx.send(true);
            }
        });
    }
    let _ = SHUTDOWN.set(tx.clone());
    Ok((tx, rx))
}
pub(crate) fn receiver() -> Option<watch::Receiver<bool>> {
    SHUTDOWN.get().map(watch::Sender::subscribe)
}
pub(crate) async fn cancelled(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}
pub(crate) async fn cancelled_optional(rx: &mut Option<watch::Receiver<bool>>) {
    match rx {
        Some(rx) => cancelled(rx).await,
        None => std::future::pending().await,
    }
}
pub(crate) async fn backoff(delay: Duration) {
    let mut rx = receiver();
    tokio::select! {
        _ = cancelled_optional(&mut rx) => {},
        _ = tokio::time::sleep(delay) => {},
    }
}

// One harness invocation owns every ACP child it ever starts, including failed
// init/respawn and dropped tasks. Only explicit supported completion removes it.
static UNCONFIRMED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(crate) fn child_spawned() {
    UNCONFIRMED.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}
pub(crate) fn child_confirmed() {
    UNCONFIRMED.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
}
pub(crate) fn write_receipt(keys: &nostr::Keys, relay: &str, run: &str) -> anyhow::Result<()> {
    use std::io::Write;
    if UNCONFIRMED.load(std::sync::atomic::Ordering::Acquire) != 0 {
        return Ok(());
    }
    let Some(path) = std::env::var_os("BUZZ_STOP_RECEIPT_PATH") else {
        return Ok(());
    };
    let receipt = buzz_core::owned_stop::sign(keys, relay, run).map_err(anyhow::Error::msg)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(serde_json::to_string(&receipt)?.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_is_sticky_for_late_subscribers() {
        let (tx, _rx) = watch::channel(false);
        tx.send(true).unwrap();
        let mut late = tx.subscribe();
        tokio::time::timeout(Duration::from_millis(20), cancelled(&mut late))
            .await
            .unwrap();
    }
}
