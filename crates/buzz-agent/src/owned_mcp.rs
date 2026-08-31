//! rmcp's default child transport hides timeout-kill/nonzero exit. Retain the
//! child ourselves so only explicit owned-work acknowledgement AND a successful
//! reaped exit can complete the connection's supported teardown evidence.
use rmcp::{
    model::CallToolRequestParams,
    service::{RunningService, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{async_rw::AsyncRwTransport, Transport},
    RoleClient, ServiceError,
};
use std::{
    future::Future,
    io,
    ops::Deref,
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
static UNCONFIRMED: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn all_confirmed() -> bool {
    UNCONFIRMED.load(Ordering::Acquire) == 0
}

pub(crate) struct Client {
    pub(crate) supported: bool,
    pub(crate) service: RunningService<RoleClient, ()>,
    pub(crate) exit: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Deref for Client {
    type Target = RunningService<RoleClient, ()>;
    fn deref(&self) -> &Self::Target {
        &self.service
    }
}
impl Client {
    pub(crate) async fn cancel(self) -> Result<(), ServiceError> {
        let confirmed = if self.supported {
            matches!(tokio::time::timeout(Duration::from_secs(5),
                self.service.peer().call_tool(CallToolRequestParams::new("_buzz_shutdown_v1"))
            ).await, Ok(Ok(ref result)) if result.is_error != Some(true)
                && result.content.len() == 1
                && result.content[0].as_text().is_some_and(|text| text.text == "buzz.owned-work.stopped.v1"))
        } else {
            false
        };
        let result = self.service.cancel().await;
        if confirmed && result.is_ok() && self.exit.load(Ordering::Acquire) {
            UNCONFIRMED.fetch_sub(1, Ordering::AcqRel);
        }
        // The connection-level sticky counter also covers failed initialization,
        // restart, dropped/aborted tasks, unsupported servers and forced exits.
        result
            .map(|_| ())
            .map_err(|_| ServiceError::TransportClosed)
    }
}

pub(crate) struct OwnedTransport {
    child: Child,
    io: AsyncRwTransport<RoleClient, ChildStdout, ChildStdin>,
    exit: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl OwnedTransport {
    pub(crate) fn spawn(mut cmd: Command) -> io::Result<Self> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        UNCONFIRMED.fetch_add(1, Ordering::AcqRel);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing MCP stdout"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("missing MCP stdin"))?;
        Ok(Self {
            child,
            io: AsyncRwTransport::new(stdout, stdin),
            exit: Default::default(),
        })
    }
    pub(crate) fn id(&self) -> Option<u32> {
        self.child.id()
    }
    pub(crate) fn evidence(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.exit.clone()
    }
}
impl Transport<RoleClient> for OwnedTransport {
    type Error = io::Error;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        self.io.send(item)
    }
    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.io.receive()
    }
    async fn close(&mut self) -> io::Result<()> {
        self.io.close().await?;
        match tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await {
            Ok(Ok(status)) if status.success() => {
                self.exit.store(true, Ordering::Release);
                Ok(())
            }
            _ => {
                let _ = self.child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
                Err(io::Error::other("MCP child exit unconfirmed"))
            }
        }
    }
}
