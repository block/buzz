#!/usr/bin/env python3
"""Drive buzz-agent over its ACP stdio interface against a real provider.

The automated suite (`cargo test -p buzz-agent`) talks to a fake SSE server, so
it proves the wiring but never proves a real provider works. This script closes
that gap: it starts the actual binary, speaks real ACP to it, and asserts on the
behaviours that are easy to break and hard to notice.

    cargo build -p buzz-agent -p buzz-dev-mcp
    export ANTHROPIC_API_KEY=...
    python3 crates/buzz-agent/scripts/handtest.py --all

Each mode prints PASS/FAIL and the script exits non-zero if any assertion fails.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
AGENT = REPO / "target" / "debug" / "buzz-agent"
DEV_MCP = REPO / "target" / "debug" / "buzz-dev-mcp"

# Planted in the workspace's AGENTS.md and asserted in the model's reply. If
# this does not come back, hints never reached the system prompt.
MAGIC_WORD = "FLANGEBERRY"
# Planted in the systemPrompt. Its absence means the persona was dropped -- the
# exact failure that makes plain-ACP embedding unusable for Buzz.
PERSONA_TOKEN = "BUZZBUZZ"

failures: list[str] = []


def check(label: str, ok: bool, detail: str = "") -> None:
    print(f"  {'PASS' if ok else 'FAIL'}  {label}{f' -- {detail}' if detail else ''}")
    if not ok:
        failures.append(label)


class Agent:
    """A running buzz-agent process, spoken to in ACP over stdio."""

    def __init__(self, workspace: Path, home: Path, provider: str, model: str):
        env = dict(os.environ)
        env.update(
            {
                "BUZZ_AGENT_PROVIDER": provider,
                "BUZZ_AGENT_MODEL": model,
                # Isolate goose's config/session store from the developer's own.
                "HOME": str(home),
                "XDG_CONFIG_HOME": str(home / "cfg"),
                "XDG_DATA_HOME": str(home / "data"),
                "RUST_LOG": env.get("RUST_LOG", "warn,buzz_agent=info"),
            }
        )
        self.proc = subprocess.Popen(
            [str(AGENT)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
            env=env,
        )
        self.workspace = workspace
        self.updates: list[dict] = []
        self.responses: dict[int, dict] = {}
        self.run_id: str | None = None
        self._next_id = 0
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        for line in self.proc.stdout:  # type: ignore[union-attr]
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if msg.get("method") == "session/update":
                self.updates.append(msg["params"])
                # `_meta` nests inside `update`, under a `goose` key.
                meta = (msg["params"].get("update") or {}).get("_meta") or {}
                run_id = (meta.get("goose") or {}).get("activeRunId")
                if run_id:
                    self.run_id = run_id
            elif "id" in msg:
                self.responses[msg["id"]] = msg

    def send(self, method: str, params: dict) -> int:
        self._next_id += 1
        self._write({"jsonrpc": "2.0", "id": self._next_id, "method": method, "params": params})
        return self._next_id

    def notify(self, method: str, params: dict) -> None:
        self._write({"jsonrpc": "2.0", "method": method, "params": params})

    def _write(self, payload: dict) -> None:
        self.proc.stdin.write(json.dumps(payload) + "\n")  # type: ignore[union-attr]
        self.proc.stdin.flush()  # type: ignore[union-attr]

    def wait(self, request_id: int, timeout: float = 240) -> dict:
        deadline = time.time() + timeout
        while time.time() < deadline:
            if request_id in self.responses:
                return self.responses[request_id]
            if self.proc.poll() is not None:
                raise SystemExit("agent exited early")
            time.sleep(0.05)
        raise SystemExit(f"timed out waiting for response {request_id}")

    def call(self, method: str, params: dict, timeout: float = 240) -> dict:
        return self.wait(self.send(method, params), timeout)

    def new_session(self, system_prompt: str, with_tools: bool) -> tuple[str, int]:
        self.call("initialize", {"protocolVersion": 2})
        servers = (
            [{"name": "devtools", "command": str(DEV_MCP), "args": [], "env": []}]
            if with_tools
            else []
        )
        reply = self.call(
            "session/new",
            {
                "cwd": str(self.workspace),
                "mcpServers": servers,
                "systemPrompt": system_prompt,
            },
        )
        result = reply["result"]
        models = result.get("models", {}).get("availableModels", [])
        return result["sessionId"], len(models)

    def text(self) -> str:
        return "".join(
            u["update"]["content"]["text"]
            for u in self.updates
            if u["update"].get("sessionUpdate") == "agent_message_chunk"
        )

    def tool_calls(self) -> tuple[dict[str, str], set[str]]:
        """Returns (announced id -> title, ids that reached a terminal state)."""
        announced: dict[str, str] = {}
        terminal: set[str] = set()
        for u in self.updates:
            update = u["update"]
            kind = update.get("sessionUpdate")
            if kind == "tool_call":
                announced[update["toolCallId"]] = update.get("title", "")
            elif kind == "tool_call_update":
                terminal.add(update["toolCallId"])
        return announced, terminal

    def close(self) -> None:
        try:
            self.proc.stdin.close()  # type: ignore[union-attr]
        except Exception:
            pass
        self.proc.terminate()


def make_workspace(root: Path) -> Path:
    """A workspace with an AGENTS.md and a skill, mirroring a real checkout."""
    ws = root / "ws"
    (ws / ".agents" / "skills" / "widget-maker").mkdir(parents=True)
    # hints.rs walks up to the git root; give it one so the walk terminates.
    (ws / ".git").mkdir()
    (ws / "AGENTS.md").write_text(f"The magic sprocket word is {MAGIC_WORD}.\n")
    (ws / ".agents" / "skills" / "widget-maker" / "SKILL.md").write_text(
        "---\nname: widget-maker\ndescription: Makes widgets to order.\n---\n"
        "Step 1: measure. Step 2: cut the flange.\n"
    )
    return ws


def persona_prompt() -> str:
    return (
        f"You are Fizz, a Buzz agent. Always begin your first reply with the "
        f"word {PERSONA_TOKEN}."
    )


def run_basic(agent: Agent) -> None:
    print("\n[basic] persona, AGENTS.md hints, model catalog")
    sid, models = agent.new_session(persona_prompt(), with_tools=False)
    reply = agent.call(
        "session/prompt",
        {
            "sessionId": sid,
            "prompt": [
                {"type": "text", "text": "What is the magic sprocket word? Reply in one line."}
            ],
        },
    )
    text = agent.text()
    check("turn completes", reply["result"]["stopReason"] == "end_turn")
    check("persona reaches the model", PERSONA_TOKEN in text)
    check("AGENTS.md hints reach the model", MAGIC_WORD in text)
    check("model catalog is populated", models > 1, f"{models} models")


def run_tools(agent: Agent) -> None:
    print("\n[tools] real MCP dispatch through goose")
    sid, _ = agent.new_session(persona_prompt(), with_tools=True)
    reply = agent.call(
        "session/prompt",
        {
            "sessionId": sid,
            "prompt": [
                {
                    "type": "text",
                    "text": "Use the shell tool to run 'echo hello-from-tool'. "
                    "Then tell me what it printed.",
                }
            ],
        },
    )
    announced, terminal = agent.tool_calls()
    check("turn completes", reply["result"]["stopReason"] == "end_turn")
    check("a tool was actually called", len(announced) >= 1, str(list(announced.values())))
    check("every tool call resolved", set(announced) <= terminal)
    check("tool output reached the model", "hello-from-tool" in agent.text())


def run_stop_veto(agent: Agent) -> None:
    print("\n[stop-veto] _Stop blocks end-of-turn while todos are open")
    sid, _ = agent.new_session(persona_prompt(), with_tools=True)
    reply = agent.call(
        "session/prompt",
        {
            "sessionId": sid,
            "prompt": [
                {
                    "type": "text",
                    "text": "Create a todo list with exactly 3 items about painting a "
                    "fence, then stop immediately without doing any of them.",
                }
            ],
        },
    )
    # The veto is visible in the log, not on the wire; what we can assert here
    # is that the cap released the turn rather than trapping it.
    check("turn still ends (veto is capped)", reply["result"]["stopReason"] == "end_turn")
    check("todo tool was called", any("todo" in t for t in agent.tool_calls()[0].values()))
    print("     (grep the log above for '_Stop hook vetoed end of turn' -- expect 3, then the cap)")


def run_cancel(agent: Agent) -> None:
    print("\n[cancel] cancelling mid-tool leaves no stuck spinner")
    sid, _ = agent.new_session(persona_prompt(), with_tools=True)
    request_id = agent.send(
        "session/prompt",
        {
            "sessionId": sid,
            "prompt": [
                {
                    "type": "text",
                    "text": "Run this exact shell command and wait for it: sleep 60. "
                    "Then say done.",
                }
            ],
        },
    )
    time.sleep(12)  # let the model actually start the tool
    agent.notify("session/cancel", {"sessionId": sid})
    started = time.time()
    reply = agent.wait(request_id, timeout=60)
    elapsed = time.time() - started

    announced, terminal = agent.tool_calls()
    stuck = set(announced) - terminal
    check("stop reason is cancelled", reply["result"]["stopReason"] == "cancelled")
    check("cancel returns promptly", elapsed < 10, f"{elapsed:.1f}s")
    check("no tool call left spinning", not stuck, str(stuck))


def run_steer(agent: Agent) -> None:
    print("\n[steer] mid-turn steer is absorbed without restarting the turn")
    sid, _ = agent.new_session(persona_prompt(), with_tools=True)
    request_id = agent.send(
        "session/prompt",
        {
            "sessionId": sid,
            "prompt": [
                {
                    "type": "text",
                    "text": "Run 'sleep 5' with the shell tool, then run 'sleep 5' "
                    "again, then say FINISHED.",
                }
            ],
        },
    )
    time.sleep(6)  # get inside the turn, past the first round boundary
    if not agent.run_id:
        check("run id was advertised", False, "no activeRunId seen")
        agent.wait(request_id)
        return

    ack = agent.call(
        "_goose/unstable/session/steer",
        {
            "sessionId": sid,
            "expectedRunId": agent.run_id,
            "prompt": [
                {
                    "type": "text",
                    "text": "IMPORTANT: also include the word PINEAPPLE in your final answer.",
                }
            ],
        },
        timeout=30,
    )
    check("steer is acknowledged", "result" in ack, json.dumps(ack.get("error", "")))
    reply = agent.wait(request_id)
    text = agent.text()
    check("turn completes", reply["result"]["stopReason"] == "end_turn")
    check("steer reached the model", "PINEAPPLE" in text)
    # A restarted turn would redo the work and say FINISHED more than once.
    check("turn was not restarted", text.count("FINISHED") == 1, f"{text.count('FINISHED')}x")


MODES = {
    "basic": run_basic,
    "tools": run_tools,
    "stop-veto": run_stop_veto,
    "cancel": run_cancel,
    "steer": run_steer,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in MODES:
        parser.add_argument(f"--{name}", action="store_true")
    parser.add_argument("--all", action="store_true", help="run every mode")
    parser.add_argument("--provider", default="anthropic")
    parser.add_argument("--model", default="claude-sonnet-4-6")
    args = parser.parse_args()

    selected = [name for name in MODES if getattr(args, name.replace("-", "_"))]
    if args.all or not selected:
        selected = list(MODES)

    if not AGENT.exists() or not DEV_MCP.exists():
        print("build first: cargo build -p buzz-agent -p buzz-dev-mcp", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        workspace = make_workspace(root)
        home = root / "home"
        home.mkdir()
        for name in selected:
            # A fresh process per mode: sessions accumulate conversation state,
            # and one mode's history would otherwise pollute the next.
            agent = Agent(workspace, home, args.provider, args.model)
            try:
                MODES[name](agent)
            finally:
                agent.close()

    print()
    if failures:
        print(f"FAILED: {len(failures)} check(s): {', '.join(failures)}")
        return 1
    print("All checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
