# Replying from a notification

A [mobile push notification](../../CONTEXT.md#mobile-push-notification) that shows a message offers a [notification reply](../../CONTEXT.md#notification-reply): the user types a reply in the notification, and the app sends it without opening.

The reply goes in a new subthread rooted at the notified message: its parent is that message, and its thread root is the message's own thread root, or the message itself when it is top-level. A Solo chat takes no thread replies, so there the reply goes at the same level as the notified message, the chat's top level.

The reply mentions the notified message's sender, in every kind of conversation including DMs and Solo chats, so the person or agent being answered is notified. A sender who has since left the conversation can't be mentioned, so the reply goes without the mention rather than failing. A notification has no mention autocomplete, so it adds no other mentions; text typed as `@name` stays plain text.

After the reply is accepted, the app marks the conversation read up to the notified message and removes the delivered notifications it covers, because replying means the user has seen that message. Newer messages stay unread: the user may not have seen them. When the reply can't be sent (offline, a channel the user isn't in, no time left), the app shows a "Reply not sent" notification instead; tapping it opens the subthread the reply would have gone in, with the typed text in its composer after any draft already there, so nothing typed is lost. A recovered draft doesn't carry the automatic mention. iOS keeps each reply on disk from the moment it's typed until the app reports what happened to it, so the text survives even when the app never got to the reply, or was stopped partway: the next launch makes it a draft, without sending it.

A reply has one deadline, set when iOS hands it to the app, about 25 seconds later. The app checks the deadline at the moment the message goes out, after any wait including a rate limit's, and publishes only while there's still time to hear the relay's answer before it. So a reply reported as not sent was never published, or the relay refused it, and retrying its draft can't post it twice. A reply that was published but got no answer may have gone out: the app says so ("Reply may not have sent") and keeps no draft.

Only a notification that shows a resolved channel or DM message offers Reply. A fallback notification that shows no message (for example "Open Buzz to view this message") offers none, whatever the reason for the fallback. There is no other per-account rule. Forum posts and comments don't offer Reply yet: a forum reply is a different kind of event with its own threading rules.

The reply is sent by the same app code as a reply typed in the app, so mentions, read state and drafts behave the same. That code runs only while the app's Flutter engine runs, and on iOS the engine starts only once the app creates its screen, which a background launch for a notification action doesn't do. So a reply typed while the app isn't running ends as "Reply not sent", and its text becomes a draft when the app next opens. The app closes its relay connection soon after going to the background, so a reply reopens it and waits a bounded time for it before sending. The connection stays open until the reply finishes, within a fixed limit, and then closes after the usual background grace period.

For now, only notifications from the community selected in the app offer Reply. Replying must not switch the selected community, and the app can currently reach only the selected community, so a notification from another one offers none. If the selection changes between the notification arriving and the reply being sent, the reply is treated as not sent, and its text is kept in the drafts of the community it came from. Ideally the selected community would decide only what the interface shows, not which writes are possible; replying in other communities can follow once the app can reach them without switching.

## Considered options

- **Reply at the same level as the notified message.** Rejected: a reply to a top-level message would then land in the channel's main flow, where it can read as a new topic and reach people who weren't part of the exchange. A subthread keeps it attached to the message it answers.
- **No automatic mention, typed `@name` resolved against channel members.** Rejected: typing an exact name without autocomplete is error-prone, and the person being answered is nearly always the one who should be notified.
- **Opening the app to send.** Rejected: it loses the main benefit of replying from the notification.
- **Sending from native code, without Flutter.** Rejected: it would duplicate the app's send, membership, read-state and draft logic in a second language for the sake of a background wake that the app's Flutter engine can handle once it's started.

## Consequences

A reply sent from a notification always notifies the sender of the message it answers, even in a DM, where the sender would be notified anyway.
