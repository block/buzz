//! Manual/e2e harness: `cargo run -p buzz-remote-access --example serve -- <member-pubkey-hex> <origin> [port]`
//! Starts the loopback listener with real shell terminals enabled and prints `listening 127.0.0.1:<port>`.
//! `RA_MGMT=fake` also enables agent management backed by an in-memory agent list (so the web UI can be driven end to
//! end); then the stdin lines `agents off` / `agents on` flip "Allow remote agent management", like the desktop switch.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use buzz_remote_access::{ManagementBackend, PtyBackend, RemoteServer, ServerConfig, TerminalMode};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

struct FakeAgents(Mutex<Vec<Value>>);

impl FakeAgents {
    fn new() -> Self {
        let mk = |n: &str, c: &str, s: &str| json!({"pubkey": c.repeat(64), "name": n, "status": s, "runtime": "goose", "provider": "anthropic", "model": "sonnet"});
        Self(Mutex::new(vec![mk("Alpha", "a", "running"), mk("Bravo", "b", "stopped"), mk("Charlie", "c", "stopped")]))
    }
}

impl ManagementBackend for FakeAgents {
    fn call(&self, op: String, p: Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>> {
        let mut agents = self.0.lock().unwrap();
        let pk = p["pubkey"].as_str().unwrap_or("").to_string();
        let r = (|| -> Result<Value, String> {
            let idx = agents.iter().position(|a| a["pubkey"] == pk);
            match op.as_str() {
                "agents.list" => Ok(json!({"agents": *agents})),
                "providers.list" => Ok(json!({"runtimes":[{"id":"goose","label":"Goose","available":true},{"id":"claude","label":"Claude Code","available":true},{"id":"gone","label":"Gone","available":false}]})),
                "personas.list" => Ok(json!({"personas":[{"id":"p1","name":"Helper"}]})),
                "agent.start" | "agent.stop" => {
                    let i = idx.ok_or("unknown_agent: That agent isn't on this computer any more.")?;
                    agents[i]["status"] = if op == "agent.start" { "running".into() } else { "stopped".into() };
                    Ok(json!({}))
                }
                "agent.message" => idx.map(|_| json!({})).ok_or_else(|| "unknown_agent: no such agent".to_string()),
                "agent.create" => {
                    let name = p["name"].as_str().unwrap_or("").trim().to_string();
                    if name.is_empty() {
                        return Err("bad_request: Name is required.".into());
                    }
                    let id = format!("{:x}", agents.len() + 10).repeat(64).chars().take(64).collect::<String>();
                    let a = json!({"pubkey": id, "name": name, "status": if p["start"] == true {"running"} else {"stopped"},
                        "runtime": p["runtimeId"], "provider": p["provider"], "model": p["model"], "personaId": p["personaId"]});
                    agents.push(a.clone());
                    Ok(json!({"agent": a}))
                }
                "agent.rename" | "agent.update" => {
                    let i = idx.ok_or("unknown_agent: That agent isn't on this computer any more.")?;
                    let running = agents[i]["status"] == "running";
                    for k in ["name", "provider", "model"] {
                        if p.get(k).is_some() {
                            agents[i][k] = p[k].clone();
                        }
                    }
                    if let Some(r) = p["runtimeId"].as_str() {
                        agents[i]["runtime"] = r.into();
                    }
                    Ok(json!({"agent": agents[i], "restartNeeded": running && op == "agent.update"}))
                }
                _ => Err("unknown_op: That operation isn't supported.".into()),
            }
        })();
        drop(agents);
        Box::pin(async move { r })
    }
}

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let member = nostr::PublicKey::parse(&a[1]).expect("member pubkey");
    let mut cfg = ServerConfig::new(member, "dev-desktop", vec![a[2].clone()]);
    cfg.backend = Some(Arc::new(PtyBackend::new(if std::env::var("RA_HERDR").is_ok() { TerminalMode::Herdr { bin: "herdr".into() } } else { TerminalMode::Shell })));
    let fake = std::env::var("RA_MGMT").map(|v| v == "fake").unwrap_or(false);
    if fake {
        cfg.mgmt = Some(Arc::new(FakeAgents::new()));
    }
    let srv = Arc::new(RemoteServer::start(cfg, a.get(3).and_then(|p| p.parse().ok()).unwrap_or(0)).await.unwrap());
    srv.set_terminals_enabled(true);
    if fake {
        srv.set_agents_enabled(true);
        let s = srv.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                match l.trim() {
                    "agents off" => s.set_agents_enabled(false),
                    "agents on" => s.set_agents_enabled(true),
                    _ => {}
                }
            }
        });
    }
    println!("listening {}", srv.addr);
    tokio::signal::ctrl_c().await.ok();
}
