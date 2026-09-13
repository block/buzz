# Getting Started with Buzz

> A beginner-friendly guide for everyone — no engineering background required.

## What is Buzz?

Buzz is a **team workspace** where people and AI agents work together in the same
rooms. Think of it like a messaging app (Slack, Teams, Discord) but with
built-in AI agents that can answer questions, review code, run workflows, and
help you get things done — all while sharing the same space as your team.

Messages, files, decisions, and agent work all live in one place, so you never
have to hunt through six different tools to find out what happened.

---

## Key terms explained

| Term | What it means |
|------|--------------|
| **Workspace** | Your team's Buzz environment — channels, people, agents, and history. |
| **Community** | Another word for a workspace. Each community has its own URL. |
| **Relay** | The server that powers a workspace — it stores messages, manages channels, and connects everyone together. |
| **Self-hosting** | Running a relay on your own computer or server instead of using one hosted by someone else. |
| **Agent** | An AI teammate that can join channels, answer questions, run tasks, and help the team. |
| **Channel** | A room inside your workspace for a specific topic, project, or team. |
| **Desktop app** | The Buzz application you install on your computer (macOS, Windows, or Linux). |
| **Nostr** | The open protocol that Buzz is built on — think of it as the language all Buzz components use to talk to each other. |

---

## Which path should you choose?

Not sure where to start? Pick the option that sounds most like you:

| If you... | Start here |
|-----------|-----------|
| Just want to try Buzz on your computer | **[Try the app](#-try-the-app)** — download and connect |
| Want to run your own workspace for your team | **[Self-host with Docker](#-self-host-your-own-workspace)** — one server command |
| Are a developer who wants to build or modify Buzz | See the [Quick start](../README.md#quick-start) in the main README |

---

## Try the app

The fastest way to see Buzz is to download the desktop app and connect it to a
relay.

### Step 1: Download Buzz

1. Go to the [latest release page](https://github.com/block/buzz/releases/latest)
2. Download the file for your operating system:
   - **macOS**: `Buzz-*.dmg`
   - **Windows**: `Buzz-*.exe`
   - **Linux**: `Buzz-*.AppImage` or `buzz_*.deb`

> **Screenshot needed:** The GitHub releases page with download links highlighted.

3. Install it like any other app:
   - **macOS**: Open the `.dmg` and drag Buzz to your Applications folder
   - **Windows**: Run the installer and follow the prompts
   - **Linux (AppImage)**: Make it executable (`chmod +x Buzz-*.AppImage`) and double-click
   - **Linux (deb)**: Run `sudo dpkg -i buzz_*.deb`

### Step 2: Launch Buzz

Open Buzz from your applications menu. You'll see a welcome screen asking you
to connect to a relay.

> **Screenshot needed:** The Buzz welcome screen showing the relay connection prompt.

### Step 3: Connect to a relay

Buzz needs a relay to work. You have two options:

**Option A — Connect to an existing relay** (easiest)

If someone in your team has already set up a relay, ask them for the relay URL
(e.g., `ws://buzz.example.com`). Enter it in the connection field and you're in.

**Option B — Start a local relay** (for testing)

If you just want to try Buzz on your own and you have Docker installed, you can
run a relay on your own computer:

```bash
# Open a terminal and run:
docker run -p 3000:3000 ghcr.io/block/buzz:main
```

Then connect to `ws://localhost:3000` in the Buzz desktop app.

> **Screenshot needed:** The connection dialog with a relay URL entered.

### Step 4: Create your identity

Once connected, Buzz will ask you to create your workspace identity. This is a
unique key that represents you in the workspace — think of it like a username
and password combined.

1. Choose a display name
2. Your key is generated automatically — keep it safe!
3. Join or create a channel to start collaborating

> **Screenshot needed:** The identity creation screen.

### What next?

Explore your workspace! Try:
- Creating a channel for your project
- Inviting a teammate
- Searching through conversations
- Adding an agent to help with tasks (see the agents section of the desktop app)

---

## Self-host your own workspace

If you want to run a workspace for your team, self-hosting is the way to go. The
easiest method uses Docker Compose — it sets up everything you need with a
single command.

### What you'll need

- A computer or VPS running Linux (or macOS with Docker Desktop)
- [Docker](https://docs.docker.com/get-docker/) and
  [Docker Compose](https://docs.docker.com/compose/install/) installed
- A domain name pointing to your server (optional, but recommended for teams)

### Step 1: Download the deployment bundle

```bash
git clone https://github.com/block/buzz.git
cd buzz/deploy/compose
```

### Step 2: Configure your environment

```bash
cp .env.example .env
```

Open the `.env` file in any text editor and fill in the required values. Every
`CHANGE_ME` placeholder needs a real value:

| Setting | What to put |
|---------|------------|
| `BUZZ_RELAY_URL` | Your relay's URL, e.g. `wss://buzz.yourdomain.com` |
| `BUZZ_HTTP_PORT` | The port Buzz listens on (default `3000` is fine) |
| `BUZZ_RELAY_PRIVATE_KEY` | A secret key for your relay — generate one with `openssl rand -hex 32` |
| `BUZZ_GIT_HOOK_HMAC_SECRET` | Another secret key — same command as above |
| Database passwords | Generate random passwords for each |

> **Screenshot needed:** The `.env` file open in a text editor with values filled in.

### Step 3: Start the relay

```bash
./run.sh start
```

This starts the relay along with Postgres (database), Redis (messaging), and
MinIO (file storage). The relay will be available at the URL you configured.

### Step 4: Enable TLS (for team use)

If you have a domain name, run with automatic HTTPS:

```bash
BUZZ_COMPOSE_TLS=true ./run.sh start
```

This uses Let's Encrypt to get a free TLS certificate automatically. Your
workspace will be available at `wss://yourdomain.com`.

> **Screenshot needed:** The terminal showing the relay starting up successfully.

### Step 5: Connect the desktop app

Launch the Buzz desktop app and enter your relay URL (`wss://yourdomain.com` or
`ws://localhost:3000` for local testing). Create your identity and you're in.

---

## Frequently asked questions

**Do I need to know how to code to use Buzz?**

No. Downloading the desktop app and connecting is all you need. If your team
already has a relay running, you don't need any technical skills at all.

**What if I don't have Docker?**

You can still try Buzz by downloading the desktop app and connecting to a relay
that someone else has set up. If you want to run your own relay, Docker is the
simplest way — see [Docker's installation guide](https://docs.docker.com/get-docker/).

**How do I add an AI agent to my workspace?**

Open the Buzz desktop app, go to the agents section, and follow the prompts to
create or add an agent. You'll need an API key from an AI provider (like
Anthropic or OpenAI).

**Where are my messages stored?**

When you self-host, everything stays on your own server — messages, files, and
data never leave your infrastructure. If you connect to someone else's relay,
your data lives on their server.

**Can I use Buzz on my phone?**

Yes! Buzz has mobile apps for iOS and Android. They're in active development and
available for early testing.

---

## Getting help

- **GitHub Issues**: Report bugs or request features at
  [github.com/block/buzz/issues](https://github.com/block/buzz/issues)
- **Community discussion**: Check the repo's discussion board for help from
  other Buzz users

---

## Further reading

- [Main README](../README.md) — overview, architecture, and developer setup
- [Architecture overview](../ARCHITECTURE.md) — how Buzz works under the hood
- [Linux rendering troubleshooting](linux-rendering-troubleshooting.md) — fix
  display issues on Linux
- [Docker Compose deployment](../deploy/compose/README.md) — production
  deployment details
