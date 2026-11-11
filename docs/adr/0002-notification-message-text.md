# Notification message text

A [mobile push notification](../../CONTEXT.md#mobile-push-notification) for a message shows the message's own text as its [notification body](../../CONTEXT.md#notification-body), derived the same way on every platform. The body keeps the message's line breaks, the text of code, and bare URLs. It drops the Markdown syntax around them: code fences and their language, inline-code backticks, the `[…](…)` of links and images (keeping the link text or image alt text), and `**` and `~~` outside code. A link label or a bold run may contain code, a fenced block closes only at a fence at least as long as its opening fence that ends its line, so backticks inside its code don't end it, and a fence with no closing fence runs to the end of the text, as in CommonMark. Runs of spaces collapse outside code, and three or more line breaks become one blank line. Other Markdown (quotes, lists, headings, single `*` or `_`) stays as typed, because stripping it would also mangle ordinary text such as `snake_case` and code identifiers.

The body is cut only where the platform would otherwise cut it. iOS shows at most 256 UTF-16 code units of a body, even when the notification is expanded, and cuts the rest without an ellipsis, often mid-word. So the body is at most 256 UTF-16 code units: a longer one keeps whole characters up to 255 units and ends with "…", so the reader can tell there is more. A platform that shows more of a body than iOS may raise its own cap to what it shows, but never lower it below that. The body is built from at most the first 4096 UTF-16 code units of the content (whole characters), which bounds the work for very long messages; when that cut drops text, the body ends with "…" too.

The cases in [`test-fixtures/notification-body.json`](../../test-fixtures/notification-body.json) are the contract. Each platform's notification code runs them in its tests.

## Considered options

- **No cap of our own.** Rejected because iOS would then cut long messages silently mid-word, which reads as the whole message.
- **A short preview with code and links replaced by `[code]` and `[link]`.** This was the previous design: one line of at most 180 characters. Expanding a notification showed the same stub, so a long message could only be read by opening the app.
- **A custom notification view (a Notification Content Extension on iOS).** Needed to show more than the platform's own limit, or formatted text. It is a separate, larger change that builds on this body rather than replacing it.

## Consequences

Notifications can show code and URLs that a message contains, as the message itself does, subject to the user's own system setting for showing previews on a locked device.

iOS also limits an expanded notification's height, so a body made of many short lines can be cut by height before it reaches 256 code units.
