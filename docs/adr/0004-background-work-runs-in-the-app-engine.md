---
status: proposed
---

# Background work runs in the app's own Flutter engine

When iOS wakes the mobile app in the background, for example for a [quick send](../../CONTEXT.md#quick-send), the work runs in the app's normal Dart code, in the same Flutter engine the window uses. The app's existing engine setup already supports this: a cold background launch by an App Intent starts the engine and runs `main()` before the intent's `perform()` asks Dart for anything, with no window and no change to how the engine starts. On an iPhone SE running a release build, Dart answered the intent 19 ms after `perform()` began.

## Considered options

- **Starting the engine earlier, before any window.** Unnecessary: the engine already starts on a background launch.
- **A second, lightweight engine for background work.** Rejected because it runs in its own isolate. While the app is also open, both would hold their own relay session and caches for the same community, for example its channel list and local pending messages, and could race, and avoiding that needs cross-engine coordination.
- **Reimplementing the background work natively.** Rejected because even a quick send needs the community's authenticated relay session, finding or opening the Bestie DM, and the same message tags the app writes. Duplicating that in Swift would have to stay wire-compatible with the Dart implementation.

## Consequences

A background launch still builds the widget tree, so background work can live in a widget that Home always builds and act through providers. It must not assume the app is in the foreground: once the app has been in the background past its grace period, the relay session has closed its socket, so background work reopens the session for its duration and then lets the background pause start over. A background wake pays the cost of starting the app's Dart code.
