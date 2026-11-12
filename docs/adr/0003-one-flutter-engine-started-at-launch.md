---
status: proposed
---

# One Flutter engine, started at launch

On iOS the mobile app creates its single Flutter engine when the app launches, before any window exists, and the window attaches to that engine when one appears. A [quick send](../../CONTEXT.md#quick-send), or other work iOS wakes the app for in the background, then runs the app's normal Dart code with no window. Before this, the engine started with the window, so a background launch with no window ran no Dart.

## Considered options

- **A second, lightweight engine for background work.** Rejected because it runs in its own isolate. While the app is also open, both would write the same durable state (for example a Solo chat's creation journal and the synced Solo grouping) and could race, and avoiding that needs cross-engine coordination.
- **Reimplementing the background work natively.** Rejected for quick send because starting a Solo chat is several relay writes with a resumable journal and encrypted synced state. Duplicating that in Swift would have to stay wire-compatible with the Dart implementation.

## Consequences

Dart startup must tolerate having no window: work that needs one waits until it appears. A background wake pays the cost of starting the app's Dart code.
