//! Same-host authenticated client for one runtime generation.

use std::{io, path::Path, time::Duration};
use tokio::{net::TcpStream, time::timeout};
use uuid::Uuid;

use crate::{
    artifacts::{process_matches_marker, read_runtime_receipt, ArtifactError},
    protocol::{
        AssignmentRecord, AssignmentSetStateRequest, Capability, ControlOperation, ControlPayload,
        ControlRequest, ControlResponse, JobId, JobListFilter, JobLogs, JobStartRequest, JobStatus,
        RuntimeReceipt, RuntimeStatus, SecretToken, CONTROL_DEADLINE_SECS,
        CONTROL_PROTOCOL_VERSION, MAX_ASSIGNMENT_TEXT_BYTES, MAX_CONTROL_REQUEST_BYTES,
        MAX_CONTROL_RESPONSE_BYTES,
    },
    server::{read_bounded_frame, write_bounded_frame, ServerError},
};

/// Cloneable generation-fenced local runtime client.
#[derive(Clone)]
pub struct RuntimeClient {
    address: std::net::SocketAddr,
    runtime_id: String,
    generation: Uuid,
    capability: Capability,
    token: SecretToken,
}
impl std::fmt::Debug for RuntimeClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeClient")
            .field("address", &self.address)
            .field("runtime_id", &self.runtime_id)
            .field("generation", &self.generation)
            .field("capability", &self.capability)
            .field("token", &self.token)
            .finish()
    }
}

impl RuntimeClient {
    /// Loads an owner-only schema-v2 receipt and completes authenticated hello.
    pub async fn from_receipt(
        path: impl AsRef<Path>,
        capability: Capability,
    ) -> Result<Self, ClientError> {
        let receipt = read_runtime_receipt(path.as_ref())?;
        Self::from_validated_receipt(&receipt, capability).await
    }

    /// Builds a client from an already loaded receipt and completes authenticated hello.
    pub async fn from_validated_receipt(
        receipt: &RuntimeReceipt,
        capability: Capability,
    ) -> Result<Self, ClientError> {
        receipt
            .validate()
            .map_err(|_| ClientError::InvalidReceipt)?;
        if !process_matches_marker(receipt.pid, &receipt.process_start_marker) {
            return Err(ClientError::InvalidReceipt);
        }
        let token = match capability {
            Capability::Controller => receipt.controller_token.clone(),
            Capability::Model => receipt.model_token.clone(),
        };
        let client = Self {
            address: receipt.control_addr,
            runtime_id: receipt.runtime_id.clone(),
            generation: receipt.generation,
            capability,
            token,
        };
        match client.call(ControlOperation::Hello).await? {
            ControlPayload::Hello(hello)
                if hello.runtime_id == client.runtime_id
                    && hello.generation == client.generation =>
            {
                Ok(client)
            }
            _ => Err(ClientError::InvalidReceipt),
        }
    }

    /// Returns local runtime status.
    pub async fn status(&self) -> Result<RuntimeStatus, ClientError> {
        match self.call(ControlOperation::Status).await? {
            ControlPayload::Status(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Lists jobs matching the local filter.
    pub async fn jobs_list(&self, filter: JobListFilter) -> Result<Vec<JobStatus>, ClientError> {
        match self.call(ControlOperation::JobsList(filter)).await? {
            ControlPayload::Jobs(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Starts a local durable job and returns after accepted state is committed.
    pub async fn jobs_start(&self, request: JobStartRequest) -> Result<JobStatus, ClientError> {
        request
            .validate()
            .map_err(|error| ClientError::InvalidRequest(error.to_string()))?;
        match self.call(ControlOperation::JobsStart(request)).await? {
            ControlPayload::Job(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Returns one local job status.
    pub async fn jobs_status(&self, job_id: JobId) -> Result<JobStatus, ClientError> {
        match self.call(ControlOperation::JobsStatus { job_id }).await? {
            ControlPayload::Job(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Requests cancellation of one verified local job tree.
    pub async fn jobs_cancel(&self, job_id: JobId) -> Result<JobStatus, ClientError> {
        match self.call(ControlOperation::JobsCancel { job_id }).await? {
            ControlPayload::Job(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Returns an independently byte- and line-bounded local log tail.
    pub async fn jobs_logs(
        &self,
        job_id: JobId,
        tail_lines: Option<u16>,
    ) -> Result<JobLogs, ClientError> {
        match self
            .call(ControlOperation::JobsLogs { job_id, tail_lines })
            .await?
        {
            ControlPayload::Logs(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Updates the exact current assignment through the generation-scoped model capability.
    pub async fn assignment_set_state(
        &self,
        assignment_id: impl Into<String>,
        request: AssignmentSetStateRequest,
    ) -> Result<AssignmentRecord, ClientError> {
        let assignment_id = assignment_id.into();
        if assignment_id.trim().is_empty() || assignment_id.len() > MAX_ASSIGNMENT_TEXT_BYTES {
            return Err(ClientError::InvalidRequest(
                "assignment id is required".into(),
            ));
        }
        request
            .validate()
            .map_err(|error| ClientError::InvalidRequest(error.to_string()))?;
        match self
            .call(ControlOperation::AssignmentSetState {
                assignment_id,
                request,
            })
            .await?
        {
            ControlPayload::Assignment(value) => Ok(value),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Requests privileged runner reconciliation.
    pub async fn reconcile(&self) -> Result<(), ClientError> {
        self.require_controller()?;
        match self.call(ControlOperation::Reconcile).await? {
            ControlPayload::Ack => Ok(()),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
    /// Stops the exact authenticated generation.
    pub async fn shutdown(&self) -> Result<(), ClientError> {
        self.require_controller()?;
        match self.call(ControlOperation::Shutdown).await? {
            ControlPayload::Ack => Ok(()),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    async fn call(&self, operation: ControlOperation) -> Result<ControlPayload, ClientError> {
        let request = ControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            generation: self.generation,
            control_token: self.token.clone(),
            operation,
        };
        let bytes = serde_json::to_vec(&request)?;
        if bytes.len() > MAX_CONTROL_REQUEST_BYTES {
            return Err(ClientError::RequestTooLarge);
        }
        let deadline = Duration::from_secs(CONTROL_DEADLINE_SECS);
        let mut stream = timeout(deadline, TcpStream::connect(self.address))
            .await
            .map_err(|_| ClientError::Timeout)??;
        timeout(
            deadline,
            write_bounded_frame(&mut stream, &bytes, MAX_CONTROL_REQUEST_BYTES),
        )
        .await
        .map_err(|_| ClientError::Timeout)??;
        let response = timeout(
            deadline,
            read_bounded_frame(&mut stream, MAX_CONTROL_RESPONSE_BYTES),
        )
        .await
        .map_err(|_| ClientError::Timeout)??;
        let response: ControlResponse = serde_json::from_slice(&response)?;
        if response.protocol_version != CONTROL_PROTOCOL_VERSION {
            return Err(ClientError::UnexpectedResponse);
        }
        if let Some(error) = response.error {
            return Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            });
        }
        response.result.ok_or(ClientError::UnexpectedResponse)
    }

    fn require_controller(&self) -> Result<(), ClientError> {
        if self.capability != Capability::Controller {
            return Err(ClientError::Unauthorized);
        }
        Ok(())
    }
}

/// Runtime client failure with no secret-bearing variants.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("runtime receipt failed: {0}")]
    Artifact(#[from] ArtifactError),
    #[error("invalid runtime receipt or handshake")]
    InvalidReceipt,
    #[error("invalid local job request: {0}")]
    InvalidRequest(String),
    #[error("control request exceeds 64 KiB")]
    RequestTooLarge,
    #[error("control operation is unauthorized")]
    Unauthorized,
    #[error("control operation timed out")]
    Timeout,
    #[error("control IO failed: {0}")]
    Io(#[from] io::Error),
    #[error("control framing failed: {0}")]
    Frame(#[from] ServerError),
    #[error("control JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control server returned {code}: {message}")]
    Remote { code: String, message: String },
    #[error("control server returned an unexpected response")]
    UnexpectedResponse,
}
