use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::sdk_err;
use buzz_core::kind::KIND_KANBAN_BOARD;
use buzz_sdk::{build_kanban_board, KanbanBoardMeta, KanbanColumnDef};
use serde_json::{json, Value};
use uuid::Uuid;

fn self_hex(client: &BuzzClient) -> String {
    client.keys().public_key().to_hex()
}

async fn query_events(client: &BuzzClient, filter: &Value) -> Result<Vec<Value>, CliError> {
    let raw = client.query(filter).await?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

/// First value of a named tag on an event.
pub(crate) fn tag(ev: &Value, name: &str) -> Option<String> {
    let tags = ev.get("tags").and_then(|t| t.as_array())?;
    for t in tags {
        let a = t.as_array()?;
        if a.first().and_then(|s| s.as_str()) == Some(name) {
            return a.get(1).and_then(|s| s.as_str()).map(String::from);
        }
    }
    None
}

/// All values of a named tag on an event.
fn multi_tag(ev: &Value, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(tags) = ev.get("tags").and_then(|t| t.as_array()) {
        for t in tags {
            if let Some(a) = t.as_array() {
                if a.first().and_then(|s| s.as_str()) == Some(name) {
                    if let Some(v) = a.get(1).and_then(|s| s.as_str()) {
                        out.push(v.to_string());
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn columns(ev: &Value) -> Vec<KanbanColumnDef> {
    let mut cols = Vec::new();
    if let Some(tags) = ev.get("tags").and_then(|t| t.as_array()) {
        for t in tags {
            if let Some(a) = t.as_array() {
                if a.first().and_then(|s| s.as_str()) == Some("column") {
                    let id = a.get(1).and_then(|s| s.as_str()).unwrap_or_default().to_string();
                    // Columns are emitted as key/value pairs after the colid:
                    //   ["column", id, "name", <name>, "wip", <n>, "order", <n>]
                    // or (no WIP limit) the shorter:
                    //   ["column", id, "name", <name>, "order", <n>]
                    // Parse by key, never by fixed index — the 6-element no-wip
                    // form would otherwise misread the trailing order value as wip.
                    let mut name = String::new();
                    let mut wip = None;
                    let mut order = 0u32;
                    let mut i = 2;
                    while i + 1 < a.len() {
                        match a[i].as_str() {
                            Some("name") => name = a[i + 1].as_str().unwrap_or_default().to_string(),
                            Some("wip") => {
                                wip = a[i + 1].as_str().and_then(|s| s.parse().ok())
                            }
                            Some("order") => {
                                order = a[i + 1]
                                    .as_str()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0)
                            }
                            _ => {}
                        }
                        i += 2;
                    }
                    cols.push(KanbanColumnDef { id, name, wip, order });
                }
            }
        }
    }
    cols.sort_by_key(|c| c.order);
    cols
}

/// Fetch the signed-in user's board event + resolved column meta.
async fn fetch_board(client: &BuzzClient, board_id: &str) -> Result<(Value, KanbanBoardMeta), CliError> {
    let me = self_hex(client);
    let filter = json!({ "kinds": [KIND_KANBAN_BOARD], "authors": [me], "limit": 1000 });
    let events = query_events(client, &filter).await?;
    let ev = events
        .into_iter()
        .find(|e| tag(e, "d").as_deref() == Some(board_id))
        .ok_or_else(|| CliError::Usage(format!("board {board_id} not found for this key")))?;
    let meta = KanbanBoardMeta {
        columns: columns(&ev),
        channels: multi_tag(&ev, "h"),
        invites: multi_tag(&ev, "invite"),
    };
    Ok((ev, meta))
}

fn new_colid() -> String {
    format!("col-{}", &Uuid::new_v4().simple().to_string()[..8])
}

fn parse_columns(s: &str) -> Result<Vec<(String, Option<u32>)>, CliError> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once(':') {
            Some((name, wip)) => {
                let wip: u32 = wip
                    .trim()
                    .parse()
                    .map_err(|_| CliError::Usage(format!("invalid WIP limit in {part:?}")))?;
                out.push((name.trim().to_string(), Some(wip)));
            }
            None => out.push((part.to_string(), None)),
        }
    }
    if out.is_empty() {
        return Err(CliError::Usage("no columns specified".into()));
    }
    Ok(out)
}

fn template_columns(name: &str) -> Vec<(&'static str, Option<u32>)> {
    match name {
        "kanban" => vec![("Backlog", Some(5)), ("In Progress", Some(3)), ("Review", None), ("Done", None)],
        "sprint" => vec![("To Do", Some(5)), ("In Progress", Some(3)), ("Blocked", None), ("Done", None)],
        "sales" => vec![("Lead", None), ("Qualified", None), ("Proposal", None), ("Won", None), ("Lost", None)],
        _ => vec![("Backlog", None)], // blank
    }
}

/// Submit a signed Kanban event and confirm the relay treated it as
/// authoritative. Like `buzz mem`, the relay returns `{accepted, message}`
/// where `message` starts with `"duplicate:"` when a NIP-33 write was rejected
/// as already-superseded by a newer head (LWW). We surface that as a
/// `Conflict` — CLI exit code 5 — instead of lying about success.
pub(crate) async fn submit_lww_event(
    client: &BuzzClient,
    event: nostr::Event,
) -> Result<serde_json::Value, CliError> {
    let raw = client.submit_event(event).await?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e} ({raw})")))?;
    let accepted = parsed
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let message = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    if message.starts_with("duplicate:") || message == "duplicate" {
        return Err(CliError::Conflict(
            "relay reported event as duplicate / dominated by a newer head".into(),
        ));
    }
    Ok(parsed)
}

async fn submit_board(
    client: &BuzzClient,
    board_id: &str,
    owner: &str,
    name: &str,
    content: &str,
    meta: &KanbanBoardMeta,
) -> Result<(), CliError> {
    let builder = build_kanban_board(board_id, owner, name, content, meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    submit_lww_event(client, event).await?;
    Ok(())
}

fn board_content(name: &str, description: Option<&str>) -> String {
    match description {
        Some(d) if !d.trim().is_empty() => format!("## {name}\n\n{}", d.trim()),
        _ => format!("## {name}"),
    }
}

pub async fn cmd_create_board(
    client: &BuzzClient,
    name: &str,
    description: Option<&str>,
    columns: Option<&str>,
    template: Option<&str>,
) -> Result<(), CliError> {
    let board_id = Uuid::new_v4().to_string();
    let owner = self_hex(client);
    let cols_raw: Vec<(String, Option<u32>)> = match columns {
        Some(cs) => parse_columns(cs)?,
        None => template_columns(template.unwrap_or("blank"))
            .into_iter()
            .map(|(n, w)| (n.to_string(), w))
            .collect(),
    };
    let meta_cols: Vec<KanbanColumnDef> = cols_raw
        .iter()
        .enumerate()
        .map(|(i, (n, w))| KanbanColumnDef {
            id: new_colid(),
            name: n.clone(),
            wip: *w,
            order: i as u32,
        })
        .collect();
    let content = board_content(name, description);
    let meta = KanbanBoardMeta { columns: meta_cols.clone(), channels: vec![], invites: vec![] };
    let _ = submit_board(client, &board_id, &owner, name, &content, &meta).await?;
    let cols_json: Vec<Value> = meta_cols
        .iter()
        .map(|c| json!({ "id": c.id, "name": c.name, "wip": c.wip, "order": c.order }))
        .collect();
    println!("{}", json!({ "board_id": board_id, "columns": cols_json, "owner": owner }));
    Ok(())
}

pub async fn cmd_get_board(client: &BuzzClient, board_id: &str) -> Result<(), CliError> {
    let (ev, _) = fetch_board(client, board_id).await?;
    println!("{ev}");
    Ok(())
}

pub async fn cmd_list_boards(client: &BuzzClient) -> Result<(), CliError> {
    let me = self_hex(client);
    let filter = json!({ "kinds": [KIND_KANBAN_BOARD], "authors": [me], "limit": 1000 });
    let events = query_events(client, &filter).await?;
    println!("{}", json!(events));
    Ok(())
}

pub async fn cmd_update_board(
    client: &BuzzClient,
    board_id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<(), CliError> {
    let (ev, meta) = fetch_board(client, board_id).await?;
    let owner = tag(&ev, "p").unwrap_or_else(|| self_hex(client));
    let cur_name = tag(&ev, "name").unwrap_or_default();
    let new_name = name.unwrap_or(&cur_name);
    let content = match description {
        Some(d) if !d.trim().is_empty() => board_content(new_name, Some(d)),
        _ => ev.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string(),
    };
    let _ = submit_board(client, board_id, &owner, new_name, &content, &meta).await?;
    println!("{}", json!({ "board_id": board_id, "name": new_name }));
    Ok(())
}

pub async fn cmd_add_column(
    client: &BuzzClient,
    board_id: &str,
    name: &str,
    wip: Option<u32>,
) -> Result<(), CliError> {
    if name.trim().is_empty() {
        return Err(CliError::Usage("column name must not be empty".into()));
    }
    let (ev, mut meta) = fetch_board(client, board_id).await?;
    let owner = tag(&ev, "p").unwrap_or_else(|| self_hex(client));
    let next_order = meta.columns.len() as u32;
    meta.columns.push(KanbanColumnDef {
        id: new_colid(),
        name: name.to_string(),
        wip,
        order: next_order,
    });
    let cur_name = tag(&ev, "name").unwrap_or_default();
    let content = ev.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let _ = submit_board(client, board_id, &owner, &cur_name, &content, &meta).await?;
    println!("{}", json!({ "board_id": board_id, "column": { "name": name, "wip": wip, "order": next_order } }));
    Ok(())
}

pub async fn cmd_rename_column(
    client: &BuzzClient,
    board_id: &str,
    colid: &str,
    new_name: &str,
) -> Result<(), CliError> {
    if new_name.trim().is_empty() {
        return Err(CliError::Usage("column name must not be empty".into()));
    }
    let (ev, mut meta) = fetch_board(client, board_id).await?;
    let owner = tag(&ev, "p").unwrap_or_else(|| self_hex(client));
    let mut found = false;
    for c in &mut meta.columns {
        if c.id == colid {
            c.name = new_name.to_string();
            found = true;
            break;
        }
    }
    if !found {
        return Err(CliError::Usage(format!("column {colid} not found on board {board_id}")));
    }
    let cur_name = tag(&ev, "name").unwrap_or_default();
    let content = ev.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let _ = submit_board(client, board_id, &owner, &cur_name, &content, &meta).await?;
    println!("{}", json!({ "board_id": board_id, "column": colid, "name": new_name }));
    Ok(())
}

pub async fn dispatch(cmd: crate::BoardsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::BoardsCmd;
    match cmd {
        BoardsCmd::Create { name, description, columns, template } => {
            cmd_create_board(client, &name, description.as_deref(), columns.as_deref(), template.as_deref()).await
        }
        BoardsCmd::Get { board } => cmd_get_board(client, &board).await,
        BoardsCmd::List => cmd_list_boards(client).await,
        BoardsCmd::Update { board, name, description } => {
            cmd_update_board(client, &board, name.as_deref(), description.as_deref()).await
        }
        BoardsCmd::AddColumn { board, name, wip } => cmd_add_column(client, &board, &name, wip).await,
        BoardsCmd::RenameColumn { board, column, name } => {
            cmd_rename_column(client, &board, &column, &name).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Mirrors the SDK's emitted column tags (builders.rs build_kanban_board):
    //   with wip: ["column", id, "name", <name>, "wip", <n>, "order", <n>]
    //   no wip:   ["column", id, "name", <name>, "order", <n>]
    fn make_board_event(col_tags: Vec<Vec<&str>>) -> Value {
        let tags: Vec<Value> = col_tags.into_iter().map(|t| json!(t)).collect();
        json!({ "tags": tags })
    }

    #[test]
    fn columns_parses_wip_and_no_wip_by_key() {
        let ev = make_board_event(vec![
            vec!["column", "col-a", "name", "Backlog", "wip", "5", "order", "0"],
            vec!["column", "col-b", "name", "In Progress", "wip", "3", "order", "1"],
            vec!["column", "col-c", "name", "Review", "order", "2"],
            vec!["column", "col-d", "name", "Done", "order", "3"],
        ]);
        let cols = columns(&ev);
        assert_eq!(cols.len(), 4);
        assert_eq!(
            cols.iter().map(|c| c.order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "no-wip columns must keep their real order (regression: fixed-index parser dropped it to 0)"
        );
        let c = &cols[2];
        assert_eq!(c.id, "col-c");
        assert_eq!(c.name, "Review");
        assert_eq!(c.wip, None, "no-wip column must not leak the order value into wip");
        assert_eq!(cols[3].wip, None);
        assert_eq!(cols[0].wip, Some(5));
        assert_eq!(cols[1].wip, Some(3));
    }

    #[test]
    fn columns_roundtrips_after_rename_shape() {
        // Same shape a rename writes back: mixed wip/no-wip columns.
        let ev = make_board_event(vec![
            vec!["column", "col-a", "name", "Backlog", "wip", "5", "order", "0"],
            vec!["column", "col-b", "name", "Code Review", "wip", "3", "order", "1"],
            vec!["column", "col-c", "name", "Done", "order", "2"],
        ]);
        let cols = columns(&ev);
        assert_eq!(cols[1].name, "Code Review");
        assert_eq!(cols[1].order, 1);
        assert_eq!(cols[2].order, 2);
        assert_eq!(cols[2].wip, None);
    }
}

