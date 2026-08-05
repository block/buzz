//! One-shot OpenClaw enrollment provider bundled with Buzz Desktop.
//!
//! The normal path uses a one-time enrollment code. An explicit host opts into
//! the SSH fallback. OpenClaw subsequently connects directly to Buzz using the
//! relay details in the v1 payload; this process is never a runtime proxy.

use serde_json::{json, Value};
use std::io::Read;
use std::process::{Command, Stdio};

fn info() -> Value {
    json!({
        "ok": true,
        "name": "openclaw",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": 1,
        "description": "Enrolls agents with OpenClaw using a one-time code",
        "config_schema": {
            "type": "object",
            "properties": {
                "enrollment_code": { "type": "string", "title": "One-time enrollment code", "description": "Paste the code shown by OpenClaw" },
                "rooms": { "type": "string", "format": "buzz-room-picker", "description": "Buzz rooms selected in Desktop" },
                "host": { "type": "string", "title": "SSH host (advanced)", "description": "Optional SSH destination, e.g. openclaw@agent-host" },
                "port": { "type": "string", "description": "Optional SSH port" }
            },
            "required": ["enrollment_code", "rooms"]
        },
        "enrollment": {
            "operation": "enroll",
            "one_time": true,
            "credential_fields": ["private_key_nsec", "auth_tag", "relay_url"]
        }
    })
}

fn error(message: impl Into<String>) -> Value {
    json!({"ok": false, "error": message.into()})
}

fn enroll(request: &Value) -> Value {
    let config = match request.get("provider_config").and_then(Value::as_object) {
        Some(config) => config,
        None => return error("provider_config must be an object"),
    };
    let code = match config
        .get("enrollment_code")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(code) => code,
        None => return error("provider_config.enrollment_code is required"),
    };
    if config.get("rooms").and_then(Value::as_str).is_none() {
        return error("provider_config.rooms is required");
    }
    let agent = match request.get("agent") {
        Some(agent) if agent.is_object() => agent,
        _ => return error("agent payload is required"),
    };

    let host = config
        .get("host")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let mut args: Vec<String> = if host.is_some() {
        vec!["-o".into(), "BatchMode=yes".into()]
    } else {
        Vec::new()
    };
    if let Some(host) = host {
        if let Some(port) = config
            .get("port")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            args.extend(["-p".into(), port.into()]);
        }
        args.push(host.into());
    }
    args.extend([
        "openclaw".into(),
        "buzz".into(),
        "enroll".into(),
        "--code".into(),
        code.into(),
        "--stdin".into(),
    ]);

    // Tests may provide a fake ssh executable. Production uses the user's
    // normal ssh trust/agent configuration for the explicit host fallback.
    let command = if host.is_some() {
        std::env::var_os("BUZZ_OPENCLAW_SSH").unwrap_or_else(|| "ssh".into())
    } else {
        "openclaw".into()
    };
    let mut child = match Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return error("could not start OpenClaw enrollment command"),
    };
    let remote_payload = json!({
        "version": 1,
        "agent": agent,
        "rooms": config.get("rooms").and_then(Value::as_str).unwrap_or_default(),
    });
    let payload = match serde_json::to_vec(&remote_payload) {
        Ok(mut payload) => {
            payload.push(b'\n');
            payload
        }
        Err(_) => return error("could not encode enrollment payload"),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if std::io::Write::write_all(&mut stdin, &payload).is_err() {
            return error("could not send enrollment payload");
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(_) => return error("OpenClaw enrollment failed"),
    };
    if !output.status.success() {
        return error("OpenClaw enrollment failed");
    }
    let response: Value = match serde_json::from_slice(&output.stdout) {
        Ok(response) => response,
        Err(_) => return error("OpenClaw returned invalid enrollment response"),
    };
    if response.get("ok") != Some(&Value::Bool(true)) {
        return error("OpenClaw enrollment was rejected");
    }
    match response
        .get("agent_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(agent_id) => json!({"ok": true, "agent_id": agent_id}),
        None => error("OpenClaw enrollment response missing agent_id"),
    }
}

fn respond(request: Value) -> Value {
    match request.get("op").and_then(Value::as_str) {
        Some("info") => info(),
        Some("enroll")
            if request
                .get("enrollment")
                .and_then(|value| value.get("version"))
                .and_then(Value::as_u64)
                == Some(1) =>
        {
            enroll(&request)
        }
        Some("enroll") => error("unsupported enrollment version"),
        _ => error("unsupported operation"),
    }
}

fn main() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        std::process::exit(1);
    }
    let response = serde_json::from_str::<Value>(&input)
        .map(respond)
        .unwrap_or_else(|_| error("request is not valid JSON"));
    println!("{}", response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_declares_one_time_v1_enrollment() {
        assert_eq!(info()["protocol_version"], 1);
        assert_eq!(info()["enrollment"]["one_time"], true);
    }

    #[test]
    fn info_prefers_code_and_marks_rooms_for_desktop_picker() {
        assert_eq!(
            info()["config_schema"]["required"],
            json!(["enrollment_code", "rooms"])
        );
        assert_eq!(
            info()["config_schema"]["properties"]["rooms"]["format"],
            "buzz-room-picker"
        );
    }

    #[test]
    fn rejects_missing_code_without_invoking_openclaw() {
        let response = respond(json!({
            "op": "enroll",
            "enrollment": {"version": 1},
            "agent": {}
        }));
        assert_eq!(response["ok"], false);
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("enrollment_code"));
    }
}
