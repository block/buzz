# CLI instructions

Read the root [`AGENTS.md`](../../AGENTS.md) first and use
[`TESTING.md`](TESTING.md) for the live-testing runbook.

- Put agent-facing operations in `buzz-cli`; add the subcommand and client
  wiring here rather than extending the developer MCP surface.
- Treat command JSON and exit behavior as a public interface. Output differs by
  command, so preserve the documented shape and inspect `buzz --help` or nearby
  command tests before changing it. Write commands report relay acceptance;
  reads are not universally arrays.
- Global flags precede subcommands: for example,
  `buzz --format compact messages thread --channel <uuid> --event <hex>`.

Build with `cargo build --release -p buzz-cli`; the binary is
`target/release/buzz`. Run focused CLI tests and the live-testing steps in
[`TESTING.md`](TESTING.md) when changing client behavior.
