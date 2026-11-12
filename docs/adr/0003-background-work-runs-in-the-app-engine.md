---
status: proposed
---

# Background work runs in the app's own Flutter engine

When iOS wakes the mobile app in the background, for example for a [quick send](../../CONTEXT.md#quick-send), the work runs in the app's normal Dart code, in the same Flutter engine the window uses. The app's existing engine setup already supports this: a cold background launch by an App Intent starts the engine and runs `main()` before the intent's `perform()` asks Dart for anything, with no window and no change to how the engine starts. On an iPhone SE running a release build, Dart answered the intent 19 ms after `perform()` began.

## Considered options

- **Starting the engine earlier, before any window.** Unnecessary: the engine already starts on a background launch.
- **A second, lightweight engine for background work.** Rejected because it runs in its own isolate. While the app is also open, both would write the same durable state (for example a Solo chat's creation journal and the synced Solo grouping) and could race, and avoiding that needs cross-engine coordination.
- **Reimplementing the background work natively.** Rejected for quick send because starting a Solo chat is several relay writes with a resumable journal and encrypted synced state. Duplicating that in Swift would have to stay wire-compatible with the Dart implementation.

## Consequences

Background work must not depend on a window or on the widget tree having built: it reads state and acts through providers directly. A background wake pays the cost of starting the app's Dart code.
