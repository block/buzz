# buzz-pubsub

Redis-backed pub/sub, presence tracking, and typing indicators.

**Key responsibilities:**
- Cross-process event broadcasting via Redis pub/sub
- Presence tracking (who is online in which channels)
- Typing indicator propagation (agents broadcast typing too)
- Uses `deadpool-redis` for connection pooling

In a multi-process relay deployment, events published by one process are broadcast to subscribers on all processes via Redis pub/sub.

**Related:**
- [buzz-relay](buzz-relay)
- [EventPipeline](../concepts/event-pipeline)
