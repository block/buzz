# Global mentions — cross-platform survey and recommendation for Buzz

Researched August 2026 against current vendor documentation. Sources at the end.

**The finding that matters.** Every platform converged on roughly the same three
keywords, so the keyword is not the design. The real design lives in three axes
that vendors resolved *differently*, and got wrong in instructive ways:

1. **Reach** — everyone, only those currently online, or a named subset.
2. **Permission** — who is allowed to use it, and whether that scales with
   group size.
3. **Mute override** — whether it pierces a muted conversation.

Buzz has no global mentions today. Building them is therefore a chance to pick
the good answers rather than inherit anyone's mistakes.

---

## What each platform offers

| Platform | Keyword | Reach | Who may use it | Pierces mute |
|---|---|---|---|---|
| **Slack** | `@here` | Active members only | Restrictable per channel by admins | No — silent if notifications paused |
| **Slack** | `@channel` | All channel members | Restrictable per channel by admins | No |
| **Slack** | `@everyone` | Whole workspace, `#general` only | Restrictable | No |
| **Slack** | `@usergroup` | A named group, but only members already in the channel | Paid plans (Pro+) | No |
| **Discord** | `@here` | Online, non-DND members with channel access | Permission-gated | No |
| **Discord** | `@everyone` | Whole server regardless of status | Same permission as `@here` — cannot be separated | No |
| **Discord** | `@role` | Everyone holding a role | Same single permission | No |
| **Teams** | `@channel` | Channel members | Respects individual notification settings | No |
| **Teams** | `@team` | Whole team; standard channels only, not private | Team-level control | **Yes — cannot be muted** |
| **Teams** | `@general` | Everyone, via the undeletable General channel | — | Depends |
| **Teams** | `@tag` | A named tag, e.g. `@finance` | Owners, or members if permitted | No |
| **Matrix / Element** | `@room` | Whole room | Off by default; enabled per room | No |
| **WhatsApp** | `@all` | Whole group | **Admins only above 32 members** | **Yes, with a per-user opt-out** |

### Instructive failures

**Discord bundled its permissions.** `@everyone`, `@here` and `@role` sit behind
a single permission, so an admin cannot allow the polite one and forbid the loud
one. This is a long-standing, repeatedly-requested complaint, and servers work
around it with bots. Separate the permissions from day one.

**Teams made `@team` unmutable**, then left `@general` as a back door: disallow
`@teamname` and people simply reach the whole team through the General channel,
which cannot be hidden. A restriction with an obvious bypass is theatre.

**Slack's silently do nothing in threads.** Users routinely do not know this.

**WhatsApp shipped `@all` this month with the most considered design of the
set**: unrestricted in small groups, admins-only above 32 members, pierces mute
because it exists for genuinely urgent things, and offers a per-user opt-out for
people who disagree. Group size gating the permission is the idea worth stealing
— it makes the feature harmless where it is harmless and controlled where it is
not, with no configuration.

---

## Recommendation for Buzz

### Build two, not five

**`@channel`** — everyone in the channel. **`@here`** — only members currently
present, which Buzz can answer from its existing presence feature.

Skip `@everyone`. It means "whole workspace", and Buzz has *communities* with a
switcher — so its scope would be genuinely ambiguous, and the one place Slack
allows it (`#general`) has no Buzz equivalent. Two clear tools beat three where
one is confusing.

### Gate by group size, not by configuration

Follow WhatsApp. Below a threshold, anyone may use them; above it, restrict to
channel admins by default. A 5-person channel does not need governance; a
40-person one does, and nobody will configure it in advance. Buzz already has
`ChannelPermissionsSettings` to hang an override on.

Keep the two permissions **separate** — that is Discord's mistake, and it is the
single cheapest thing to get right at the start and expensive to unpick later.

### Decide mute deliberately

This is the one genuinely contested axis. Teams pierces mute and offers no
escape, which people resent. WhatsApp pierces it but lets you opt out. Slack
never pierces it.

Recommendation: **`@channel` pierces mute with a per-user opt-out; `@here` never
does.** That gives one escalation path that actually works and one polite
default, and it matches how Buzz already treats muting elsewhere — muted
channels still show an unread indicator, just no notification.

### Decide thread behaviour explicitly

Slack's broadcast mentions do nothing inside threads and users are caught out by
it. Buzz should either make them work in threads or visibly refuse them there —
silently discarding is the worst option.

### Defer user groups

Every platform has them (`@usergroup`, `@role`, `@tag`) and they are genuinely
useful, but they need group creation, membership management and a sync story.
Buzz has `community-members` and agent teams to build on later. Not first.

---

## What this costs in Buzz

Mentions today are individual only: a `["p", <pubkey>]` tag per person, with
`isHighPriorityEventForUser` treating any message tagging you as high priority.
The pieces needed:

1. **A tag convention** — a channel-wide marker rather than N `p` tags, since
   tagging 40 pubkeys individually bloats every event and breaks as membership
   changes.
2. **Autocomplete entries** for `@channel` and `@here` in `MentionAutocomplete`.
3. **Resolution at notify time** — expand the marker against current membership
   (and presence, for `@here`) in `shouldNotify`, so the audience is whoever is
   in the channel when it is *read*, not when it was sent.
4. **The permission check**, at send time and enforced at render.
5. **Timeline rendering** so the mention is visibly a broadcast.

Note this needs **no new event kind** and no relay change — it is a tag on an
ordinary message — so it clears the constraint that killed in-app WebRTC.

**One caution.** Notification fatigue was the single most common complaint about
Slack in every source reviewed. This feature is the mechanism that produces it.
Ship it with the gate, not after.

---

## Sources

- [Notify a channel or workspace — Slack](https://slack.com/help/articles/202009646-Notify-a-channel-or-workspace)
- [Manage who can notify a channel or workspace — Slack](https://slack.com/help/articles/115004855143-Manage-who-can-notify-a-channel-or-workspace)
- [Mentioning user groups — U-M TeamDynamix](https://teamdynamix.umich.edu/TDClient/30/Portal/KB/Article/8616/Slack-Mentioning-User-Groups-in-a-Workspace)
- [Setting up permissions FAQ — Discord](https://support.discord.com/hc/en-us/articles/206029707-Setting-Up-Permissions-FAQ)
- [Separate the ability to mention @here and @everyone — Discord community](https://support.discord.com/hc/en-us/community/posts/360040470172-Separate-the-ability-to-mention-here-and-everyone)
- [Use tags to @mention groups — Microsoft Support](https://support.microsoft.com/en-us/teams/teams-channels/use-tags-to-mention-groups-in-microsoft-teams)
- [@mentions explained in Microsoft Teams](https://www.m365.fm/blog/mentions-explained-in-microsoft-teams-the-complete-guide/)
- [Understanding Matrix rooms — Element docs](https://docs.element.io/latest/element-support/matrix-rooms/understanding-matrix-rooms/)
- [WhatsApp adds '@all' mentions — MacRumors, Aug 2026](https://www.macrumors.com/2026/08/04/whatsapp-adds-all-mentions-and-poll-upgrades/)
- [WhatsApp @all bypasses group mute — TechTimes, Aug 2026](https://www.techtimes.com/articles/323151/20260805/whatsapp-all-mention-bypasses-group-mute-sweeping-coordination-upgrade.htm)
