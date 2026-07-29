# Vision: your project, your domain

> **Document status:** Long-term product direction. Buzz already ships parts of
> this model, including communities, relay hosting, git transport, and the repo
> browser; the complete one-domain forge experience described here is not a
> statement that every capability is available today.

`myproject.com` is your community's workspace. It is not an account page that
happens to have your name on it. Your domain, your community, and the relay
infrastructure behind it work as one system.

The community is the workspace. Code and conversation live there. Agents connect
through its relay. Automation flows through it. Artifacts publish from it. You
type the URL and see the workspace. That's the whole idea.

Nothing here is novel on its own. Git hosting is solved. Chat is solved. Agents are
solved. Nostr identity is solved. The insight is putting them all behind one domain
with one identity system. Each piece is boring. The combination is the thing.

The examples here are about software because that's the sharpest case — code,
branches, CI, releases. But the model works for anything a group builds together.
Research, hardware, writing, governance. The relay doesn't care what's in the
channels or the repos. It stores signed events, routes messages, runs workflows,
and hosts files. What you build with that is up to you.

---

## What You See

You type `myproject.com`. You see a list of repos — like a GitHub org page, but it's
yours, running on your server, with your data. You click one. You're at
`repoa.myproject.com`. The README renders. The file tree is there. Click into `src/`
and the code is syntax-highlighted. The clone URL sits at the top:

```
git clone repoa.myproject.com
```

Next to it, a "Connect on Buzz" button.

No separate website. No static site generator, no deploy step, no "pages" concept.
The relay serves rendered HTML to browsers and responds to `git clone` at the same
URL — content negotiation, one domain, two audiences. The repo *is* the website.
A browser gets a rendered page. A git client gets a git server. Same URL.

You clone it. It works. No special tooling. No new protocol. Git is git. You push
the same way. Agents clone and push the same way. The repos are repos.

You hit "Connect on Buzz" and the app opens. You're in `#general`. The team is
talking. There's a forum with open issues and design discussions. There's a canvas
with the architecture doc — updated last week by the docs agent after a refactor
landed. Three feature branches are active, each one a channel. You're in.
Everything is here.

A new contributor finds the project the same way. They browse the code, read the
README, click into the forum and see the open issues. They read a design doc in the
canvas. They understand the project before they write a line of code. Then they hit
"Connect on Buzz" and they're in the community. No separate onboarding flow. No
"read our wiki at wiki.myproject.com and our forum at forum.myproject.com and our
chat at discord.gg/..." Everything is at `myproject.com`. That's where everything
is.

Not every project needs to run its own relay. Most people will join a hosted
**community**: one workspace at one URL, with a tenant boundary that may have a
dedicated deployment or share infrastructure with thousands of other
communities. Either way it is the same OSS codebase, and a co-tenant cannot see
your events, profiles, DMs, or search. Your key stays yours across all of them;
identity is portable even when the hosting is not. And you can use Buzz as a
collaboration layer on top of another git host if that makes sense — work in
Buzz channels, push releases to your public repo. The sovereign setup is the
full version, but the tools work at every level of commitment.

---

## How Work Happens

A contributor files a bug. It lands in the forum. The triage agent — sitting in the
channel like any other member — labels it, finds a duplicate from six weeks ago,
links the relevant docs. This happens before a human reads it. The contributor gets
a response in seconds. The maintainer sees a pre-processed report, not a raw inbox.
They read the triage summary, confirm it's real, assign it. Thirty seconds of their
time instead of five minutes.

Someone picks up the bug. They create a branch. Buzz creates a channel:
`#feat-auth-fix`. They work there. They push commits. The CI agent picks up the
work — it's watching for pushes, it's just a member with compute — runs the tests,
posts results back to the channel. Green. The patch lands in the channel as a
reviewable diff. A co-maintainer reviews it inline, leaves a comment on line 45,
approves it. The approval is a signed event — cryptographic proof of who said yes
and when.

Merge. The workflow runs: sequential integration, tests after each merge, release
event published. The channel archives. The conversation is now the permanent record
of why that code exists. Not a chat thread that scrolled away. Not a PR that might
get deleted. The event log, on your relay, in your database. Someone reading that
code in two years can pull up the channel and see exactly what was discussed, what
was tried, what was rejected, and why the final approach won.

The whole flow — bug report to merged patch — happened in one place. No
tab-switching between issue tracker, CI dashboard, chat, and code review. One
stream. One URL. One search index. You search "auth refresh" and you get the bug
report, the channel discussion, the patches, the CI results, and the design doc
that motivated the whole thing — all in one query.

**The release flow** is where it clicks. An agent in `#releases` watches `main`.
When a release is needed — triggered by a workflow, or by a human posting "ship
it" — it assembles the changelog from every merged patch since the last tag,
formats it, posts a draft. The maintainer edits two lines, approves. The workflow
kicks off: builds artifacts, pushes to content-addressed storage, deploys wherever
the project deploys. The result posts back to `#releases`. Done. The whole thing
is logged, signed, and traceable.

The release agent isn't special infrastructure. It's an npub with compute that
happens to be fast. Same for the CI agent, the triage agent, the docs agent. They
sit in channels. They watch for things. They clone repos and push code. They get
pinged by workflows or by humans. A CI agent picks up work from a branch channel,
runs tests, posts results back. A docs agent watches for merges and proposes doc
updates. A release agent watches `main` and ships. They're not a separate system.
They're contributors.

Workflows are the connective tissue. They fire when messages match filters, when
conditions are met, when a timer ticks. They don't execute work — they coordinate
it. The relay is the message bus. Agents are the workers. They run wherever they
run: your server, a cloud function, a laptop. The workflow just knows who to ping
and when. You define a workflow once at the project level and every branch channel
inherits it automatically — no per-branch configuration, no copy-pasting YAML.

---

## The Social Layer

Not everything belongs in the repo. Not everything belongs in a channel. Some
things are announcements. Some things are essays. Buzz has surfaces for both,
and they live on the same relay as the code.

Short notes — project announcements, "we just shipped X," the kind of thing you'd
post on social media — are just notes. They live on the relay, they're public, they
propagate across the nostr network. Someone following your project sees them in
their feed. No separate social media account to maintain. No "follow us on Twitter"
link pointing somewhere else. Your relay is your social presence. When you ship a
release, you post a note. It goes out to everyone following the project's npub.
That's it.

Long-form posts are for the thinking that doesn't fit anywhere else. The design
decision that deserves more than a channel message. The research write-up explaining
why you chose a particular approach. The post-mortem on that outage. The RFC that
shaped the architecture. These aren't commit messages and they're not channel
threads — they're documents that deserve to exist on their own, searchable and
permanent, alongside the code that implements them. Years later, someone reading
the code can find the reasoning. It's on the same relay. Same search. Same
identity. The blog post and the code it describes are in the same place.

Channels are the working conversation — fast, ephemeral-feeling, but actually
permanent. The repo is the code and docs that belong with the code. Notes are the
public face. Long-form is the thinking. Each surface has its job. Nothing bleeds
into the wrong place.

The result is that your project has a complete public presence at one domain. The
code, the conversation, the announcements, the deep thinking — all there, all
searchable, all yours. A new contributor can browse the repo, read the design docs,
skim the recent announcements, and join the community without leaving `myproject.com`.
A researcher studying the project's evolution can trace decisions from the long-form
posts through the channel discussions to the merged patches. It's all connected
because it's all on the same relay.

---

## Identity

Your npub is your identity everywhere. One keypair. No accounts to create on each
new platform. No "sign in with GitHub" that means GitHub owns your identity. Your
keys, your identity.

Reputation follows you across projects. When you contribute to three projects and
get patches merged, that history is on the relay — signed, verifiable, queryable.
When you show up at a fourth project, your track record shows up with you. A
maintainer can see: merged patches, vouches from people they know, a history of
good work. That's not a number a platform assigned you. It's your actual history,
cryptographically attested. You can't fake it. You can't buy it. You earn it by
doing the work.

Web-of-trust emerges naturally. Who vouches for whom. Who merges whose patches.
Who has a track record. No special reputation system to design and build — it's
the natural consequence of cryptographic identity plus public contribution history.
A maintainer you trust vouches for a contributor you've never heard of. That vouch
is a signed event. You can see it. You can weight it. The trust graph grows
organically as people work together, and it's queryable across the whole network.

This matters especially for agents. An agent with a persistent keypair and a
verifiable contribution history is fundamentally different from an anonymous
generator with no history. The agent has skin in the game. Its reputation is on
the line with every contribution, across every project it touches. Bad work
degrades its standing everywhere, not just in one repo. That's the right incentive
structure, and it falls out of the identity model for free.

You don't have to build a separate detection system for low-quality contributions.
You look at the npub's history. A fresh keypair with no history and no vouches gets
scrutiny. An npub with 50 merged patches across projects you respect gets
fast-tracked. The signal is in the history. The web-of-trust does the filtering.

No platform owns your identity. No platform can suspend your account. Your keypair
is yours. If you move to a different relay, your identity comes with you. Your
contribution history is portable. The work you did is yours. If `myproject.com`
goes down, your npub still exists. Your history still exists. You stand up a new
relay, point your domain at it, and you're back. The project continues.

---

## What You Give Up

You run infrastructure. A server, a domain, a relay. That's not hard — a modest
VPS handles a small project comfortably — but it's not zero. Someone has to keep
it running. Someone has to handle backups. Someone has to deal with the 3 AM alert
when the disk fills up. Managed hosting can take that off your plate — your
project runs as a community on shared infrastructure, isolated from every other
tenant, same sovereignty, someone else handles the ops — but it's a cost either
way, in time or money. Worth knowing before you start.

Key management is harder than "sign in with Google." Losing your private key means
losing your identity. There's no "forgot password" flow, no support ticket to file,
no account recovery. Hardware keys help. Good practices help. But it's a real
tradeoff and you should go in knowing it. The same property that makes your identity
uncensorable makes it unrecoverable if you lose the key.

The ecosystem is young. The tooling is good and getting better, but it's not a
decade of polish. Some things will feel rough. Some integrations won't exist yet.
You're early, and early means occasionally hitting edges that haven't been smoothed.

Your contributors are early too. Most developers don't have a nostr keypair.
Onboarding friction is real — not insurmountable, but real. The "Connect on Buzz"
button is easy. Explaining what a keypair is takes a sentence. But it's a sentence
you'll have to write, and some contributors won't bother. You'll lose some people
at the door who would have clicked "sign in with GitHub" without thinking. That's
a real cost, especially early in a project when you're trying to grow a contributor
base.

These are honest costs. They're worth it if you care about owning your project —
your code, your community, your data, your identity. They're not worth it if you
just want the path of least resistance. Know which one you are.

---

## The Point

One domain. One identity. Everything in one place — and it's yours.
