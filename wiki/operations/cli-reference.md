# CLI Reference

## just (task runner)

The `Justfile` at the project root provides common development commands.

### Development

| Command | Description |
|---|---|
| `just dev` | Start full dev stack (Docker Compose) |
| `just dev-reset` | Destroy and recreate dev stack |
| `just seed-local-community` | Seed sample data |
| `just desktop` | Launch desktop client dev server |
| `just mobile` | Launch Flutter dev server |

### Testing

| Command | Description |
|---|---|
| `just test-all` | Run all tests |
| `just test-rust` | Run Rust tests |
| `just test-desktop` | Run desktop E2E tests (Playwright) |
| `just test-mobile` | Run Flutter tests |

### Building

| Command | Description |
|---|---|
| `just build-relay` | Build relay binary |
| `just build-desktop` | Build desktop app |
| `just build-mobile` | Build mobile app |
| `just docker` | Build Docker images |

## cargo

Standard Cargo commands work. Key workspace crates:

```bash
cargo build -p buzz-relay
cargo test -p buzz-conformance
cargo clippy -p buzz-core
```

## Hermit

Hermit manages toolchain versions. Activate automatically by `cd`-ing into `buzz/`, or manually:

```bash
. bin/activate-hermit
```

**Related:**
- [DevelopmentSetup](development-setup)
- [Deployment](deployment)
