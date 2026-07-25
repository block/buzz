//! Platform wake notification boundary for same-day schedule catch-up.

use std::sync::Arc;

pub(crate) type WakeEventHandler = Arc<dyn Fn() + Send + Sync + 'static>;

/// Owned platform subscription. Dropping it stops further wake delivery.
pub(crate) trait WakeSubscription: Send {}

/// Testable boundary over an operating-system wake event source.
pub(crate) trait WakeEventSource {
    fn subscribe(
        &self,
        handler: WakeEventHandler,
    ) -> Result<Box<dyn WakeSubscription>, &'static str>;
}

#[cfg(target_os = "macos")]
pub(crate) struct MacWorkspaceWakeSource;

#[cfg(target_os = "macos")]
struct MacWorkspaceWakeSubscription {
    child: Arc<std::sync::Mutex<std::process::Child>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "macos")]
impl WakeSubscription for MacWorkspaceWakeSubscription {}

#[cfg(target_os = "macos")]
impl Drop for MacWorkspaceWakeSubscription {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(target_os = "macos")]
impl WakeEventSource for MacWorkspaceWakeSource {
    fn subscribe(
        &self,
        handler: WakeEventHandler,
    ) -> Result<Box<dyn WakeSubscription>, &'static str> {
        use std::io::BufRead;
        use std::process::Stdio;

        let helper = crate::command_services::apple_inputs::verified_bundled_helper_path()?;
        let mut child = std::process::Command::new(helper)
            .arg("--watch-workspace-wake")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "wake_source_unavailable")?;
        let stdout = child.stdout.take().ok_or("wake_source_unavailable")?;
        let child = Arc::new(std::sync::Mutex::new(child));
        let reader = std::thread::Builder::new()
            .name("buzz-workspace-wake".to_string())
            .spawn(move || {
                let reader = std::io::BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(line) if line == "workspace_did_wake" => handler(),
                        _ => break,
                    }
                }
            })
            .map_err(|_| "wake_source_unavailable")?;
        Ok(Box::new(MacWorkspaceWakeSubscription {
            child,
            reader: Some(reader),
        }))
    }
}

#[cfg(test)]
#[path = "wake_tests.rs"]
mod tests;
