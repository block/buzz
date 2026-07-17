//! Slim goose agent as an ACP stdio server.
//!
//! Buzz talks to agents over ACP (see `buzz-acp`); this binary lets the Buzz
//! distribution bundle a capable goose agent without requiring users to
//! install goose. It is the entire integration: goose's public ACP server on
//! stdin/stdout. We explicitly enable the in-core developer extension because
//! a fresh embedded Goose config does not otherwise guarantee shell/file tools.

fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(goose::acp::server::run(vec!["developer".to_string()]))
}
