use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::sdk_err;
use buzz_core::kanban;
use buzz_core::kind::KIND_KANBAN_BOARD;
use buzz_sdk::{build_kanban_card, KanbanCardMeta, KanbanColumnDef};
use serde_json::{json, Value};
use uuid::Uuid;

use super::boards::{columns, tag};

async fn query_events(client: &BuzzClient, filter: &Value) -> Result<Vec<Value>, CliError> {
    let raw = client.query(filter).await?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn self_hex(client: &BuzzClient) -> String {
    client.keys().public_key().to_hex()
}

/// Fetch the board event + its column meta for `board_id`.
async fn get_board(client: &BuzzClient, board_id: &str) -> Result<(Value, Vec<KanbanColumnDef>), CliError> {
    let me = self_hex(client);
    let filter = json!({ "kinds": [KIND_KANBAN_BOARD], "authors": [me], "limit": 1000 });
    let events = query_events(client, &filter).await?;
    let ev = events
        .into_iter()
        .find(|e| tag(e, "d").as_deref() == Some(board_id))
        .ok_or_else(|| CliError::Usage(format!("board {board_id} not found for this key")))?;
    let cols = columns(&ev);
    Ok((ev, cols))
}

fn board_ref(owner: &str, board_id: &str) -> String {
    format!("31001:{owner}:{board_id}")
}

/// All cards on a board (by `#a` board ref).
async fn list_cards_by_board(client: &BuzzClient, board_ref: &str) -> Result<Vec<Value>, CliError> {
    let filter = json!({ "kinds": [31002], "#a": [board_ref], "limit": 1000 });
    query_events(client, &filter).await
}

fn card_tag(ev: &Value, name: &str) -> Option<String> {
    tag(ev, name)
}

fn card_multi_tag(ev: &Value, name: &str) -> Vec<String> {
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

/// Labels are NIP-32 namespaced `["l",<label>,"kanban"]`; strip the namespace.
fn card_labels(ev: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(tags) = ev.get("tags").and_then(|t| t.as_array()) {
        for t in tags {
            if let Some(a) = t.as_array() {
                if a.first().and_then(|s| s.as_str()) == Some("l")
                    && a.get(2).and_then(|s| s.as_str()) == Some("kanban")
                {
                    if let Some(v) = a.get(1).and_then(|s| s.as_str()) {
                        out.push(v.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Rebuild a card's meta from an existing event (preserves fields not being changed).
fn meta_from_event(ev: &Value, board_ref: &str, column: Option<&str>, rank: Option<&str>) -> KanbanCardMeta {
    KanbanCardMeta {
        board: board_ref.to_string(),
        column: column.map(str::to_string).unwrap_or_else(|| card_tag(ev, "column").unwrap_or_default()),
        rank: rank.map(str::to_string).or_else(|| card_tag(ev, "rank")),
        assignees: card_multi_tag(ev, "p"),
        labels: card_labels(ev),
        due: card_tag(ev, "due"),
        thread: card_tag(ev, "e"),
    }
}

/// Highest rank string among the given cards (that are in `column`), if any.
fn max_rank(cards: &[Value], column: &str) -> Option<String> {
    cards
        .iter()
        .filter(|c| card_tag(c, "column").as_deref() == Some(column))
        .filter_map(|c| card_tag(c, "rank"))
        .max()
}

fn validate_rank(r: &str) -> Result<(), CliError> {
    if !kanban::is_valid(r) {
        return Err(CliError::Usage(format!("invalid rank {r:?}: must be a base-36 rank string not ending in '0'")));
    }
    Ok(())
}

async fn submit_card(
    client: &BuzzClient,
    card_id: &str,
    content: &str,
    meta: KanbanCardMeta,
) -> Result<(), CliError> {
    let builder = build_kanban_card(card_id, content, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    crate::commands::boards::submit_lww_event(client, event).await?;
    Ok(())
}

pub async fn cmd_create_card(
    client: &BuzzClient,
    board_id: &str,
    column: Option<&str>,
    title: &str,
    body: Option<&str>,
    labels: &[String],
    assignees: &[String],
) -> Result<(), CliError> {
    if title.trim().is_empty() {
        return Err(CliError::Usage("card title must not be empty".into()));
    }
    let (board_ev, cols) = get_board(client, board_id).await?;
    let owner = board_ev.get("pubkey").and_then(|p| p.as_str()).unwrap_or(&self_hex(client)).to_string();
    let refv = board_ref(&owner, board_id);
    let colid = match column {
        Some(c) => {
            if !cols.iter().any(|c2| c2.id == c) {
                return Err(CliError::Usage(format!("column {c} not found on board {board_id}")));
            }
            c.to_string()
        }
        None => cols
            .iter()
            .find(|c| c.order == 0)
            .or_else(|| cols.first())
            .map(|c| c.id.clone())
            .ok_or_else(|| CliError::Usage(format!("board {board_id} has no columns")))?,
    };

    let cards = list_cards_by_board(client, &refv).await?;
    let rank = match max_rank(&cards, &colid) {
        Some(r) => kanban::rank_after(&r),
        None => kanban::first_rank(),
    };

    let content = match body {
        Some(b) if !b.trim().is_empty() => format!("{title}\n\n{}", b.trim()),
        _ => title.to_string(),
    };
    let card_id = Uuid::new_v4().to_string();
    let meta = KanbanCardMeta {
        board: refv,
        column: colid.clone(),
        rank: Some(rank.clone()),
        assignees: assignees.to_vec(),
        labels: labels.to_vec(),
        due: None,
        thread: None,
    };
    let _ = submit_card(client, &card_id, &content, meta).await?;
    println!("{}", json!({ "card_id": card_id, "board": board_id, "column": colid, "rank": rank }));
    Ok(())
}

pub async fn cmd_list_cards(
    client: &BuzzClient,
    board_id: &str,
    column: Option<&str>,
) -> Result<(), CliError> {
    let (board_ev, cols) = get_board(client, board_id).await?;
    let owner = board_ev.get("pubkey").and_then(|p| p.as_str()).unwrap_or(&self_hex(client)).to_string();
    let refv = board_ref(&owner, board_id);
    let mut cards = list_cards_by_board(client, &refv).await?;
    if let Some(c) = column {
        cards.retain(|c2| card_tag(c2, "column").as_deref() == Some(c));
    }
    // sort by column order, then lexicographic rank
    let order_of = |ev: &Value| -> (usize, String) {
        let c = card_tag(ev, "column").unwrap_or_default();
        let order = cols.iter().position(|col| col.id == c).unwrap_or(usize::MAX);
        let r = card_tag(ev, "rank").unwrap_or_default();
        (order, r)
    };
    cards.sort_by(|a, b| order_of(a).cmp(&order_of(b)));
    println!("{}", json!(cards));
    Ok(())
}

pub async fn cmd_get_card(client: &BuzzClient, board_id: &str, card_id: &str) -> Result<(), CliError> {
    let (board_ev, _) = get_board(client, board_id).await?;
    let owner = board_ev.get("pubkey").and_then(|p| p.as_str()).unwrap_or(&self_hex(client)).to_string();
    let refv = board_ref(&owner, board_id);
    let cards = list_cards_by_board(client, &refv).await?;
    let ev = cards
        .into_iter()
        .find(|c| card_tag(c, "d").as_deref() == Some(card_id))
        .ok_or_else(|| CliError::Usage(format!("card {card_id} not found on board {board_id}")))?;
    println!("{ev}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_move_card(
    client: &BuzzClient,
    board_id: &str,
    card_id: &str,
    column: &str,
    rank: Option<&str>,
) -> Result<(), CliError> {
    let (board_ev, cols) = get_board(client, board_id).await?;
    if !cols.iter().any(|c| c.id == column) {
        return Err(CliError::Usage(format!("column {column} not found on board {board_id}")));
    }
    let owner = board_ev.get("pubkey").and_then(|p| p.as_str()).unwrap_or(&self_hex(client)).to_string();
    let refv = board_ref(&owner, board_id);
    let cards = list_cards_by_board(client, &refv).await?;
    let old = cards
        .iter()
        .find(|c| card_tag(c, "d").as_deref() == Some(card_id))
        .ok_or_else(|| CliError::Usage(format!("card {card_id} not found on board {board_id}")))?;

    let new_rank = match rank {
        Some(r) => {
            validate_rank(r)?;
            r.to_string()
        }
        None => match max_rank(&cards, column) {
            Some(r) => kanban::rank_after(&r),
            None => kanban::first_rank(),
        },
    };
    let content = old.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let meta = meta_from_event(old, &refv, Some(column), Some(&new_rank));
    let _ = submit_card(client, card_id, &content, meta).await?;
    println!("{}", json!({ "card_id": card_id, "board": board_id, "column": column, "rank": new_rank }));
    Ok(())
}

pub async fn cmd_update_card(
    client: &BuzzClient,
    board_id: &str,
    card_id: &str,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<(), CliError> {
    let (board_ev, _) = get_board(client, board_id).await?;
    let owner = board_ev.get("pubkey").and_then(|p| p.as_str()).unwrap_or(&self_hex(client)).to_string();
    let refv = board_ref(&owner, board_id);
    let cards = list_cards_by_board(client, &refv).await?;
    let old = cards
        .iter()
        .find(|c| card_tag(c, "d").as_deref() == Some(card_id))
        .ok_or_else(|| CliError::Usage(format!("card {card_id} not found on board {board_id}")))?;

    let old_content = old.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let content = match (title, body) {
        (Some(t), Some(b)) if !b.trim().is_empty() => format!("{t}\n\n{}", b.trim()),
        (Some(t), None) => t.to_string(),
        (None, Some(b)) if !b.trim().is_empty() => b.trim().to_string(),
        _ => old_content,
    };
    let meta = meta_from_event(old, &refv, None, None);
    let _ = submit_card(client, card_id, &content, meta).await?;
    println!("{}", json!({ "card_id": card_id, "board": board_id }));
    Ok(())
}

pub async fn dispatch(cmd: crate::CardsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::CardsCmd;
    match cmd {
        CardsCmd::Create { board, column, title, body, label, assignee } => {
            cmd_create_card(client, &board, column.as_deref(), &title, body.as_deref(), &label, &assignee).await
        }
        CardsCmd::List { board, column } => cmd_list_cards(client, &board, column.as_deref()).await,
        CardsCmd::Get { board, card } => cmd_get_card(client, &board, &card).await,
        CardsCmd::Move { board, card, column, rank } => {
            cmd_move_card(client, &board, &card, &column, rank.as_deref()).await
        }
        CardsCmd::Update { board, card, title, body } => {
            cmd_update_card(client, &board, &card, title.as_deref(), body.as_deref()).await
        }
    }
}
