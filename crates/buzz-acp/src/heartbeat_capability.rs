//! Side-effect-free capability probe used by the Desktop before a designated
//! heartbeat harness is spawned or an existing process is reused.

use anyhow::{bail, Result};
use serde::Serialize;

pub(crate) use crate::heartbeat_capability_constants::{BUILD_CAPABILITY, KIND, PROTOCOL_VERSION};

pub(crate) const COMMAND: &str = "heartbeat-preflight-capability";

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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_info_plist_scalar_is_exact_and_derived_from_runtime_constants() {
        let attestation = format!("{KIND}/v{PROTOCOL_VERSION}/{BUILD_CAPABILITY}");
        assert_eq!(
            attestation,
            "buzz_acp_heartbeat_preflight_capability/v1/\
             buzz-acp-source-witness-gateway-v1"
        );

        let plist = include_str!(concat!(
            env!("OUT_DIR"),
            "/buzz-acp-heartbeat-capability-Info.plist"
        ));
        let expected = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>BuzzHeartbeatPreflightCapability</key>\n\
             \t<string>{attestation}</string>\n\
             </dict>\n\
             </plist>\n"
        );
        assert_eq!(plist, expected);
    }
}
