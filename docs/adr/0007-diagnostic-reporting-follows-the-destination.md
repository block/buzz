# Diagnostic reporting follows the reporting destination

A mobile build does [diagnostic reporting](../../CONTEXT.md#diagnostic-reporting) if, and only if, it was built with a [reporting destination](../../CONTEXT.md#reporting-destination). The kind of build does not matter: an [internal build](../../CONTEXT.md#internal-build), a [public build](../../CONTEXT.md#public-build) or a development build reports when given a destination and is silent without one. This repository supplies none, so builds made from it alone report nothing. The app has no setting to turn reporting off and does not tell the person that it reports.

We chose this so that the open-source app carries no organization's destination and no rule about which organizations may collect reports, the same reasoning that keeps organization hosts out of the code in [ADR 0001](0001-internal-and-public-mobile-builds.md). The first use is an organization's internal build, where everyone who has the app is a member of the organization that reads the reports.

A report may carry the stack trace and error type, app, operating system and device details, a trail of recent events written from a fixed list, the host names of the servers the app talks to, the signed-in member's public key, a random identifier the app creates for its installation, a random identifier the reporting SDK creates for the installation (which stands in as the user on reports sent while no one is signed in, and appears in the SDK's session records, which also carry when a session started, its length from start to end, how it ended and its error count), on iOS native crash and hang reports only, a hash the SDK computes from the identifier iOS gives the app's vendor, the hardware model and the app's bundle identifier (it stays the same across launches, and across a reinstall while another app from the same vendor stays installed) and, when the person is signed in to a Builderlab account, that Builderlab's origin and the account's subject identifier, never its email address. Networking measurements also carry the operation's name from a fixed list, its outcome, the kind of failure and how long it took. A report never carries message content, channel or community names, private keys or other secrets, or a URL's query string or fragment.

## Considered options

- **Tie reporting to the internal build.** Rejected because the kind of build would then carry a second meaning, and a public or development build could never be diagnosed the same way.
- **Refuse to start a public build that has a reporting destination.** This would have guaranteed that people outside the distributing organization are never reported on. Rejected in favor of one rule with no exceptions; whoever supplies the destination decides where the build goes.
- **A random identifier per installation in place of the public key.** Rejected because the reports exist so that someone can follow up with the person who hit the fault. An installation identifier is now sent alongside the public key instead, so a device's reports can be matched to the members who used it.
- **A switch or a notice in Settings.** Rejected for the first use, where the people reported on are the organization's own members.

## Consequences

Because the app says nothing about reporting, any notice or consent owed to the people who receive a build is the responsibility of whoever gives that build a reporting destination. Giving a destination to a build that reaches the general public would report on those people without telling them.

What a report may carry was chosen for an internal build. Before a public build is given a reporting destination, that list has to be revisited: at the least the member's public key and the server host names would come out, and the people using the build would need to be told. Nothing in the app enforces this.

A build without a reporting destination must not start the reporting code at all, since a reporting library given an empty destination can still install its hooks.

A relay's host name can identify a community even though the community's name is never sent. We accepted this so that a failing relay can be told apart from a healthy one; an internal build talks only to its own organization's relay.

The public key ties a report to a member. Anyone with access to the reporting destination can therefore see which member hit which fault and when.
