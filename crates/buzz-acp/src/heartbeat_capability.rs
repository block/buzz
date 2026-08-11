//! Side-effect-free capability probe used by the Desktop before a designated
//! heartbeat harness is spawned or an existing process is reused.

use anyhow::{bail, Result};
use serde::Serialize;

pub(crate) const COMMAND: &str = "heartbeat-preflight-capability";
pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const BUILD_CAPABILITY: &str = "buzz-acp-source-witness-gateway-v1";
pub(crate) const KIND: &str = "buzz_acp_heartbeat_preflight_capability";

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct Capability<'a> {
    kind: &'a str,
    protocol_version: u32,
    build_capability: &'a str,
}

/// Print the exact machine capability without initializing logging, relay,
/// credentials, or an ACP/model process. Returns whether the command matched.
pub(crate) fn emit_if_requested() -> Result<bool> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).and_then(|arg| arg.to_str()) != Some(COMMAND) {
        return Ok(false);
    }
    if args.len() != 2 {
        bail!("{COMMAND} accepts no arguments");
    }
    println!(
        "{}",
        serde_json::to_string(&Capability {
            kind: KIND,
            protocol_version: PROTOCOL_VERSION,
            build_capability: BUILD_CAPABILITY,
        })?
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_schema_is_exact_and_versioned() {
        let value = serde_json::to_value(Capability {
            kind: KIND,
            protocol_version: PROTOCOL_VERSION,
            build_capability: BUILD_CAPABILITY,
        })
        .expect("serialize capability");
        assert_eq!(value["kind"], KIND);
        assert_eq!(value["protocol_version"], 1);
        assert_eq!(value["build_capability"], BUILD_CAPABILITY);
        assert_eq!(value.as_object().expect("object").len(), 3);
    }
}
