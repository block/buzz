# Buzz domain language

Shared terminology for Buzz product behavior. This glossary currently covers mobile [push suppression](#push-suppression), the [notification body](#notification-body) and [mobile builds](#mobile-builds); it is not a complete model of Buzz.

## Notifications

### Mobile push notification

A notification directed to a user's mobile installation through the platform's push service.

### Notification body

The text a [mobile push notification](#mobile-push-notification) shows for a message: the message's text with its Markdown syntax removed, cut only where the platform would cut it. See [ADR 0002](docs/adr/0002-notification-message-text.md).

### Push suppression

Withholding an otherwise eligible [mobile push notification](#mobile-push-notification) based on [desktop activity](#desktop-activity) and a working connection to the relevant community. It does not depend on whether desktop alerts can be shown.

_Avoid_: Read receipt, mark as read, mute

### Desktop activity

User interaction with the computer running Buzz, including interaction with another application. It is the interaction itself, not a state that lasts afterward, and does not imply that a particular Buzz message was displayed or read.

_Avoid_: Message visibility, read activity

### In-app activity

User interaction inside Buzz. Recent in-app activity remains evidence of [desktop activity](#desktop-activity) after Buzz loses focus; current window focus is a separate condition.

### Desktop inactivity

The state reached when the duration specified in the spec has elapsed since the last observed [desktop activity](#desktop-activity).

### Additional notification delay

A waiting period after [desktop inactivity](#desktop-inactivity) begins before [mobile push notifications](#mobile-push-notification) resume. This is distinct from the inactivity threshold itself.

### Suppressed message

A message whose otherwise eligible [mobile push notification](#mobile-push-notification) was withheld. The term refers to its notification, not withholding the message itself, and says nothing about whether the message has been read.

### Suppression activity signal

Information communicated for deciding whether [desktop activity](#desktop-activity) warrants [push suppression](#push-suppression). It is distinct from [visible presence](#visible-presence) and is not a read receipt.

### Visible presence

The availability status shown to other members, such as online, away, or offline. A chosen visible presence status does not necessarily reflect actual [desktop activity](#desktop-activity).

### Suppression renewal

A [suppression activity signal](#suppression-activity-signal) that extends how long [push suppression](#push-suppression) applies based on activity on one desktop.

### Inactivity cutoff

The time of the last observed [desktop activity](#desktop-activity) plus the duration specified in the spec, when [desktop inactivity](#desktop-inactivity) begins. New desktop activity moves this cutoff forward.

## Mobile builds

### Internal build

A mobile app an organization distributes only to its own members, for example through device management. It uses only that organization's Builderlab and reaches only that organization's community.

_Avoid_: Dogfood build, enterprise build, org build

### Public build

The mobile app distributed through the App Store and TestFlight. It uses only the public Builderlab and cannot reach any organization's community.

_Avoid_: App Store build, external build
