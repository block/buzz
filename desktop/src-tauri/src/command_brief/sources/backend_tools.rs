use super::*;

pub(crate) trait SourceToolCaller: Send + Sync {
    fn call(
        &self,
        service: &AuthenticatedSourceService,
        tool_name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, AdmissionError>;
}

#[allow(
    dead_code,
    reason = "The production orchestrator installs this native MCP caller"
)]
pub(super) struct AuthenticatedMcpSourceCaller;

impl SourceToolCaller for AuthenticatedMcpSourceCaller {
    fn call(
        &self,
        service: &AuthenticatedSourceService,
        tool_name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, AdmissionError> {
        service.call(tool_name, arguments, cancellation)
    }
}
