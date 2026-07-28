# buzz-relay

The main WebSocket relay server. Built with **Axum** on **Tokio**. Coordinates all subsystems and never exposes internal crate boundaries to clients.

**Key responsibilities:**
- WebSocket connection lifecycle (accept, auth, recv/send/heartbeat loops, cleanup)
- Event pipeline execution (12 steps)
- HTTP endpoints: `/events`, `/query`, `/count`, `/hooks/*`, `/media/*`, `/git/*`
- Connection management via `ConnectionManager` and `SubscriptionRegistry` (DashMap)
- Community binding via `TenantContext`

**Connection lifecycle:**
1. Community binding (Host header → TenantContext)
2. Semaphore acquire (connection limit)
3. NIP-42 challenge (`["AUTH", "<challenge>"]`)
4. Authentication (client signs challenge)
5. Three concurrent tasks: `recv_loop`, `send_loop`, `heartbeat_loop` (30s ping)
6. Cleanup on disconnect

**Related:**
- [Relay](../entities/relay)
- [EventPipeline](../concepts/event-pipeline)
- [buzz-core](buzz-core)
- [buzz-db](buzz-db)
