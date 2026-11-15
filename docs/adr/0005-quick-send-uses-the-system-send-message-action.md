---
status: proposed
---

# Quick send uses the system send-message action

On iOS 27 and later, [quick send](../../CONTEXT.md#quick-send) is an App Intent that adopts the system `messages.sendMessage` schema, which Siri and Shortcuts already understand. Its destination is optional and defaults to the [Bestie](../../CONTEXT.md#bestie), so letting people name another agent, person or conversation later only widens the destinations it accepts, while existing Action button setups and Siri phrases keep working. iOS 16 to 26 get a plain App Intent and App Shortcut that sends only to the Bestie.

## Considered options

- **A custom tool intent for the Siri AI Developer Preview.** Rejected: Apple's guidance is that a messaging app implementing `sendMessage` should not also mark it as a tool, and the preview works only in development-signed and internal TestFlight builds.
- **A Bestie-only intent with no destination.** Rejected because adding named recipients later would mean a second intent and new phrases.
