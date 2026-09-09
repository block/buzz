//! Reference implementation of the Buzz browser plugin contract `0.1.0-alpha`
//! (MCP protocol revision `2026-07-28`). Implements the wire shapes
//! `server/discover`, `tools/list`, and `tools/call` for the single
//! `browser.resolve` tool. See `README.md` for the published contract.
//!
//! The home URL is baked in at compile time via the `BUZZ_BROWSER_HOME_URL`
//! environment variable (set by `build.py`), not read at runtime — the
//! contract supplies no runtime home-URL environment variable.

use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2026-07-28";
const MAX_INPUT_LEN: usize = 2048;
const REJECTION_REASON: &str = "not a web address";

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if !request.is_object() {
            continue;
        }
        let id = request["id"].clone();
        let response = match request["method"].as_str() {
            Some("server/discover") => discover(id),
            Some("tools/list") => tools_list(id),
            Some("tools/call") => call(id, &request["params"]),
            _ => continue,
        };
        let Ok(text) = serde_json::to_string(&response) else {
            continue;
        };
        if writeln!(stdout, "{text}").is_err() {
            break;
        }
        if stdout.flush().is_err() {
            break;
        }
    }
}

fn discover(id: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {
            "resultType": "complete",
            "supportedVersions": [PROTOCOL_VERSION],
            "capabilities": { "tools": { "listChanged": false } },
            "cacheScope": "private",
            "ttlMs": 0
        }
    })
}

fn tools_list(id: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {
            "resultType": "complete",
            "cacheScope": "private",
            "ttlMs": 0,
            "tools": [{
                "name": "browser.resolve",
                "title": "Resolve an address",
                "description": "Turn what the user typed into a URL to open.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["context", "kind"],
                    "properties": {
                        "context": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": [
                                "contractVersion",
                                "pluginId",
                                "contributionId",
                                "sessionId",
                                "generation"
                            ],
                            "properties": {
                                "contractVersion": { "type": "string" },
                                "pluginId": { "type": "string" },
                                "contributionId": { "type": "string" },
                                "sessionId": { "type": "string" },
                                "generation": { "type": "integer", "minimum": 0 }
                            }
                        },
                        "kind": { "enum": ["home", "address"] },
                        "input": { "type": "string", "maxLength": 2048 }
                    }
                },
                "outputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["outcome"],
                    "properties": {
                        "outcome": { "enum": ["resolved", "rejected"] },
                        "url": { "type": "string", "maxLength": 2048 },
                        "title": { "type": "string", "maxLength": 200 },
                        "reason": { "type": "string", "maxLength": 200 }
                    }
                }
            }]
        }
    })
}

fn home_url() -> String {
    option_env!("BUZZ_BROWSER_HOME_URL")
        .unwrap_or("https://example.com/")
        .to_string()
}

fn call(id: serde_json::Value, params: &serde_json::Value) -> serde_json::Value {
    if params["name"].as_str() != Some("browser.resolve") {
        return serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32602, "message": "unknown tool" }
        });
    }
    let arguments = &params["arguments"];
    let outcome = match arguments["kind"].as_str() {
        Some("home") => Ok(home_url()),
        Some("address") => normalize(arguments["input"].as_str().unwrap_or("")),
        _ => Err(REJECTION_REASON.to_string()),
    };
    match outcome {
        Ok(url) => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "resultType": "complete",
                "content": [{ "type": "text", "text": url }],
                "structuredContent": { "outcome": "resolved", "url": url }
            }
        }),
        Err(reason) => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "resultType": "complete",
                "isError": true,
                "content": [{ "type": "text", "text": reason.clone() }],
                "structuredContent": { "outcome": "rejected", "reason": reason }
            }
        }),
    }
}

/// Accept a full `http`/`https` URL, or add `https://` to a host-like input.
fn normalize(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() || input.len() > MAX_INPUT_LEN || input.contains(char::is_whitespace) {
        return Err(REJECTION_REASON.to_string());
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        return Ok(input.to_string());
    }
    if input.contains("://") {
        return Err(REJECTION_REASON.to_string());
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains('.') && !host.starts_with('.') && !host.ends_with('.') {
        let url = format!("https://{input}");
        // Adding the scheme can push an input that passed the length check
        // above over the limit; the returned `url` must itself stay within
        // it (the outputSchema and the host's URL length rule both cap it
        // at 2048).
        if url.len() > MAX_INPUT_LEN {
            return Err(REJECTION_REASON.to_string());
        }
        return Ok(url);
    }
    Err(REJECTION_REASON.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discover_advertises_the_protocol_version_and_required_fields() {
        let response = discover(json!(1));
        let result = &response["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["supportedVersions"], json!([PROTOCOL_VERSION]));
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(result["cacheScope"], "private");
        assert_eq!(result["ttlMs"], 0);
    }

    #[test]
    fn tools_list_advertises_exactly_one_tool_named_browser_resolve() {
        let response = tools_list(json!(2));
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "browser.resolve");
    }

    fn call_params(kind: &str, input: Option<&str>) -> serde_json::Value {
        let mut arguments = json!({
            "context": {
                "contractVersion": "0.1.0-alpha",
                "pluginId": "dev.example.webbrowser",
                "contributionId": "web",
                "sessionId": "01J9Z0",
                "generation": 0
            },
            "kind": kind
        });
        if let Some(input) = input {
            arguments["input"] = json!(input);
        }
        json!({
            "name": "browser.resolve",
            "_meta": { "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION },
            "arguments": arguments
        })
    }

    #[test]
    fn unknown_tool_name_is_a_json_rpc_error_not_a_result() {
        let params = json!({ "name": "not.a.real.tool", "arguments": {} });
        let response = call(json!(5), &params);
        assert_eq!(response["error"]["code"], -32602);
        assert!(response.get("result").is_none());
    }

    #[test]
    fn home_resolves_to_the_compiled_in_home_url() {
        let response = call(json!(3), &call_params("home", None));
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["outcome"], "resolved");
        assert_eq!(structured["url"], home_url());
    }

    #[test]
    fn address_with_scheme_is_returned_unchanged() {
        let response = call(
            json!(3),
            &call_params("address", Some("https://example.org/docs")),
        );
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["outcome"], "resolved");
        assert_eq!(structured["url"], "https://example.org/docs");
    }

    #[test]
    fn bare_host_gets_https_added() {
        let response = call(json!(3), &call_params("address", Some("example.org/docs")));
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["outcome"], "resolved");
        assert_eq!(structured["url"], "https://example.org/docs");
    }

    #[test]
    fn empty_input_is_rejected_as_a_structured_result_not_an_error() {
        let response = call(json!(4), &call_params("address", Some("")));
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["outcome"],
            "rejected"
        );
        assert!(response.get("error").is_none());
    }

    #[test]
    fn overlong_scheme_prefixed_input_is_rejected() {
        // Already has a scheme, so this takes the early-return path with no
        // downstream length check — isolates the input-length guard itself
        // (a host-shaped no-scheme overlong input would also get caught by
        // the post-prefix check below, which wouldn't prove this guard
        // still exists).
        let input = format!("https://{}", "a".repeat(MAX_INPUT_LEN));
        assert!(input.len() > MAX_INPUT_LEN);
        let response = call(json!(4), &call_params("address", Some(&input)));
        assert_eq!(
            response["result"]["structuredContent"]["outcome"],
            "rejected"
        );
    }

    #[test]
    fn input_within_the_limit_that_only_exceeds_it_after_adding_https_is_rejected() {
        // Passes the input-length check (exactly MAX_INPUT_LEN), but
        // prefixing "https://" pushes the returned url over the limit.
        let input = format!("example.com/{}", "a".repeat(2036));
        assert_eq!(input.len(), MAX_INPUT_LEN);
        let response = call(json!(4), &call_params("address", Some(&input)));
        assert_eq!(
            response["result"]["structuredContent"]["outcome"],
            "rejected"
        );
    }

    #[test]
    fn input_with_whitespace_is_rejected() {
        let response = call(json!(4), &call_params("address", Some("example.org docs")));
        assert_eq!(
            response["result"]["structuredContent"]["outcome"],
            "rejected"
        );
    }

    #[test]
    fn unsupported_scheme_is_rejected() {
        let response = call(
            json!(4),
            &call_params("address", Some("ftp://example.com/file")),
        );
        assert_eq!(
            response["result"]["structuredContent"]["outcome"],
            "rejected"
        );
    }

    #[test]
    fn non_host_input_is_rejected() {
        let response = call(json!(4), &call_params("address", Some("not a host")));
        assert_eq!(
            response["result"]["structuredContent"]["outcome"],
            "rejected"
        );
    }
}
