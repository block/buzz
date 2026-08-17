//! Map Flow Studio canvas blocks to `buzz-workflow` YAML actions.

use buzz_workflow::schema::{ActionDef, Step};

use crate::blocks::BlockCategory;

/// Canvas block instance before YAML conversion.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CanvasBlock {
    /// Unique instance id on the canvas.
    pub id: String,
    /// Registry block type (`agent`, `http`, …).
    pub block_type: String,
    /// Block-specific config JSON.
    pub config_json: serde_json::Value,
}

/// Convert a canvas block to a workflow step (MVP mapping).
pub fn block_to_step(block: &CanvasBlock) -> Result<Step, BridgeError> {
    let action = match block.block_type.as_str() {
        "agent" => {
            let persona = block
                .config_json
                .get("persona")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            ActionDef::SendMessage {
                text: format!("Run agent persona: {persona}"),
                channel: None,
            }
        }
        "condition" => {
            let expr = block
                .config_json
                .get("expression")
                .and_then(|v| v.as_str())
                .unwrap_or("true");
            return Ok(Step {
                id: block.id.clone(),
                name: None,
                if_expr: Some(expr.to_string()),
                timeout_secs: None,
                block_type: Some(block.block_type.clone()),
                action: ActionDef::SendMessage {
                    text: "{{trigger.text}}".into(),
                    channel: None,
                },
            });
        }
        "http" => {
            let url = block
                .config_json
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BridgeError::MissingField("url".into()))?;
            ActionDef::CallWebhook {
                url: url.to_string(),
                method: block
                    .config_json
                    .get("method")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                headers: None,
                body: block
                    .config_json
                    .get("body")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }
        }
        "human_approval" => {
            let from = block
                .config_json
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("@anyone");
            let message = block
                .config_json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Approve this step?");
            ActionDef::RequestApproval {
                from: from.to_string(),
                message: message.to_string(),
                timeout: block
                    .config_json
                    .get("timeout")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }
        }
        "code" => ActionDef::SendMessage {
            text: "Code block (sandbox not wired)".into(),
            channel: None,
        },
        other => {
            return Err(BridgeError::UnknownBlockType(other.to_string()));
        }
    };

    Ok(Step {
        id: block.id.clone(),
        name: None,
        if_expr: None,
        timeout_secs: None,
        block_type: Some(block.block_type.clone()),
        action,
    })
}

/// Category label for palette grouping in YAML export metadata.
pub fn category_for_block_type(block_type: &str) -> Option<BlockCategory> {
    match block_type {
        "agent" => Some(BlockCategory::Agent),
        "condition" => Some(BlockCategory::Condition),
        "http" => Some(BlockCategory::Http),
        "code" => Some(BlockCategory::Code),
        "human_approval" => Some(BlockCategory::HumanApproval),
        _ => None,
    }
}

/// Bridge errors when converting canvas → workflow.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Unknown block registry key.
    #[error("unknown block type: {0}")]
    UnknownBlockType(String),
    /// Required config field missing.
    #[error("missing required field: {0}")]
    MissingField(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn human_approval_maps_to_request_approval() {
        let block = CanvasBlock {
            id: "gate".into(),
            block_type: "human_approval".into(),
            config_json: json!({"from": "@mgr", "message": "OK?"}),
        };
        let step = block_to_step(&block).expect("map");
        assert!(matches!(step.action, ActionDef::RequestApproval { .. }));
    }
}
