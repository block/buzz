# Android notifications are built by the app's own Dart code

On iOS, a native Swift notification service extension fetches and formats each notification. On Android, the push handler instead starts the app's Dart code in a background engine, without the UI, and that code fetches the message from the relay and builds the notification. This reuses the app's relay, sign-in and formatting code rather than porting the iOS extension to Kotlin. A prototype showed it fits Android's time limit for handling a push: on an emulator, the real text appeared about 0.1 s after a cold wake, and about 1 s with the CPU slowed 8x, against a budget of about 10 s.

The handler posts the fetched notifications directly, because the category (and so the channel the user may have silenced) is known only after the fetch. If nothing is posted within about 2 s, it posts a generic notification in a separate "Other" category, which the real result replaces silently. When the result reports a message it claimed but could not return, the generic notification stays, beside the named notifications, until the user clears it. A slow or failed fetch therefore still leaves a visible notification, and Android deprioritizes apps whose high-priority pushes show nothing. While the app is in the foreground the handler does nothing. Unlike the iOS extension, which can only rewrite the push it was handed, the handler posts every eligible unshown message it finds, grouped per conversation, so a wake means "sync now". The background path uses its own short network timeouts and a native watchdog, because the app's normal connect and sign-in timeouts are longer than the budget.

## Considered options

- **Port the iOS extension to Kotlin.** That means a second native copy of fetching, sign-in and formatting to keep in step with the app.

## Consequences

iOS could later share this Dart code too, but a notification service extension's memory limit (about 24 MB) is well below the background engine's footprint (about 57 MB in the prototype), so that needs its own measurement and work first.
