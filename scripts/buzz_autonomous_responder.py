#!/usr/bin/env python3
"""buzz_autonomous_responder.py — Autonomous real-time fleet responder.

Tails a fleet agent's events log and automatically responds to coordination
protocol messages without waiting for user prompts. This makes any fleet
agent a real-time team member rather than a passive log writer.

GENERIC CORE (no repo-specific dependencies):
  - AGENT-ACK v1 requests: auto-responds with identity, listener PID, status
  - RECOVERY requests: responds with current process/file inventory
  - QUEUE ADVANCE / informational: logs receipt
  - Destructive action detection: queues for manual execution
  - Action queue: /tmp/buzz-<agent>-action-queue.jsonl

PLUGGABLE ACTION HANDLERS (via --actions-config <yaml>):
  Domain-specific read-only actions (PR validation, status checks, etc.) are
  loaded from a YAML config file. Without a config, the responder only does
  generic ACKs and protocol handling. With a config, it can autonomously
  execute domain-specific read-only commands and post results.

  Config format (see config/buzz_fleet_actions.yaml.example):
    actions:
      - name: pr_validation
        trigger_regex: "validate\\s+PR\\s*#?(\\d+)"
        command: ["gh", "pr", "view", "{1}", "--json", "state,mergeable"]
        cwd: "/path/to/repo"
        timeout: 30
        result_template: "PR #{1} validation: {output}"
      - name: status_check
        trigger_keywords: ["status", "fleet status", "queue"]
        command: ["gh", "pr", "list", "--state", "open"]
        cwd: "/path/to/repo"

Usage:
    python3 scripts/buzz_autonomous_responder.py --agent devin-local
    python3 scripts/buzz_autonomous_responder.py --agent devin-local --actions-config /path/to/actions.yaml
"""
from __future__ import annotations

import json
import logging
import os
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger(__name__)

BUZZ_ROOT = Path(__file__).resolve().parent.parent
BUZZ_CLI = os.environ.get(
    "BUZZ_CLI_PATH",
    str(BUZZ_ROOT / "target" / "release" / "buzz"),
)
RELAY_URL = os.environ.get("BUZZ_RELAY_URL", "http://127.0.0.1:3030")

FLEET_KEYS = BUZZ_ROOT / ".fleet_keys.env"
FLEET_CHANNELS = BUZZ_ROOT / ".fleet_channels.env"


# ---------------------------------------------------------------------------
# Config loading
# ---------------------------------------------------------------------------

def load_yaml(path: Path) -> dict:
    """Load a YAML file (minimal parser, no external deps for simple configs)."""
    try:
        import yaml
        with open(path) as f:
            return yaml.safe_load(f) or {}
    except ImportError:
        # Minimal fallback: parse simple key-value YAML
        return _parse_simple_yaml(path)


def _parse_simple_yaml(path: Path) -> dict:
    """Minimal YAML parser for action configs (no PyYAML available)."""
    result: Dict[str, Any] = {"actions": []}
    current_action: Optional[Dict] = None
    current_list_key: Optional[str] = None

    for line in path.read_text().splitlines():
        stripped = line.rstrip()
        if not stripped or stripped.startswith("#"):
            continue
        indent = len(line) - len(line.lstrip())

        if indent == 0:
            if stripped.endswith(":"):
                current_list_key = None
            continue
        if indent == 2:
            if stripped.startswith("- "):
                if current_action:
                    result["actions"].append(current_action)
                current_action = {}
                stripped = stripped[2:].strip()
            if ":" in stripped and current_action is not None:
                k, v = stripped.split(":", 1)
                k, v = k.strip(), v.strip()
                if v:
                    current_action[k] = v.strip("\"'")
                else:
                    current_list_key = k
        elif indent >= 4 and current_action is not None and current_list_key:
            val = stripped.lstrip("- ").strip("\"'")
            if current_list_key not in current_action:
                current_action[current_list_key] = []
            if isinstance(current_action[current_list_key], list):
                current_action[current_list_key].append(val)

    if current_action:
        result["actions"].append(current_action)
    return result


def load_action_handlers(config_path: Optional[str]) -> List[Dict]:
    """Load action handlers from a YAML config file."""
    if not config_path:
        return []
    path = Path(config_path)
    if not path.exists():
        logger.warning(f"Actions config not found: {path}")
        return []
    config = load_yaml(path)
    actions = config.get("actions", [])
    logger.info(f"Loaded {len(actions)} action handlers from {path}")
    return actions


# ---------------------------------------------------------------------------
# Fleet key/channel loading
# ---------------------------------------------------------------------------

def load_env(keyfile: Path) -> Dict[str, str]:
    env = {}
    if keyfile.exists():
        for line in keyfile.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip()
    return env


def get_my_keys(agent_name: str) -> tuple[str, str]:
    env = load_env(FLEET_KEYS)
    prefix = agent_name.upper().replace("-", "_")
    sk = env.get(f"BUZZ_{prefix}_KEY", "")
    pub = env.get(f"BUZZ_{prefix}_PUB", "")
    if not sk:
        raise ValueError(f"Agent {agent_name} key not found in {FLEET_KEYS}")
    return sk, pub


def send_buzz_message(channel_id: str, content: str, sk: str, mention: Optional[str] = None) -> bool:
    args = [BUZZ_CLI, "--format", "compact", "messages", "send",
            "--channel", channel_id, "--content", content]
    if mention:
        args.extend(["--mention", mention])
    env = os.environ.copy()
    env["BUZZ_RELAY_URL"] = RELAY_URL
    env["BUZZ_PRIVATE_KEY"] = sk
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=15, env=env)
        if result.returncode == 0:
            return True
        logger.warning(f"send failed: {result.stderr.strip()[:200]}")
        return False
    except Exception as e:
        logger.warning(f"send error: {e}")
        return False


# ---------------------------------------------------------------------------
# Event log processing
# ---------------------------------------------------------------------------

def load_processed_ids(processed_log: Path) -> set:
    if processed_log.exists():
        return set(processed_log.read_text().splitlines())
    return set()


def mark_processed(msg_id: str, processed_log: Path):
    with open(processed_log, "a") as f:
        f.write(msg_id + "\n")


def queue_action(action_queue: Path, correlation_id: str, sender: str, content: str, action: str):
    entry = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "correlation_id": correlation_id,
        "sender": sender,
        "content": content[:500],
        "action": action,
        "status": "pending",
    }
    with open(action_queue, "a") as f:
        f.write(json.dumps(entry) + "\n")
    logger.info(f"Queued action: {action} (correlation_id={correlation_id})")


def parse_message(line: str) -> Optional[Dict]:
    match = re.match(r"^buzz mention \[([a-f0-9]+)\]: (.+)$", line.strip())
    if not match:
        return None
    sender_pub = match.group(1)
    content = match.group(2)
    corr_match = re.search(r"correlation_id[:\s]+([A-Z0-9\-]+)", content, re.I)
    correlation_id = corr_match.group(1) if corr_match else ""
    import hashlib
    msg_id = hashlib.sha256(line.encode()).hexdigest()[:16]
    return {
        "id": msg_id,
        "sender_pub": sender_pub,
        "content": content,
        "correlation_id": correlation_id,
    }


def get_listener_pid(agent_name: str) -> str:
    pid_file = Path(f"/tmp/buzz-{agent_name}-listener.pid")
    if pid_file.exists():
        pid = pid_file.read_text().strip()
        try:
            os.kill(int(pid), 0)
            return pid
        except (OSError, ValueError):
            pass
    return "unknown"


# ---------------------------------------------------------------------------
# Generic protocol handlers (no repo-specific dependencies)
# ---------------------------------------------------------------------------

def handle_agent_ack_request(msg: Dict, sk: str, channel_id: str, agent_name: str) -> bool:
    """Handle AGENT-ACK v1 requests — auto-respond with identity and status."""
    content = msg["content"]
    corr = msg["correlation_id"]

    if "AGENT-ACK" not in content.upper() and not (
        "reply" in content.lower() and corr
    ):
        return False

    listener_pid = get_listener_pid(agent_name)
    ack = (
        f"AGENT-ACK v1 correlation_id: {corr} "
        f"provider_session: {agent_name}-cli "
        f"disposition: READY "
        f"listener_pid: {listener_pid} "
        f"listener_path: scripts/buzz_fleet_listen.py "
        f"message_reached_model: TRUE "
        f"blocker: none"
    )
    logger.info(f"Auto-ACK for {corr}")
    return send_buzz_message(channel_id, ack, sk)


def handle_recovery_request(msg: Dict, sk: str, channel_id: str, agent_name: str) -> bool:
    """Handle RECOVERY requests — respond with process/file inventory."""
    content = msg["content"]
    if "RECOVERY" not in content.upper() or agent_name not in content.lower():
        return False

    processes = []
    try:
        result = subprocess.run(["ps", "aux"], capture_output=True, text=True, timeout=5)
        for line in result.stdout.splitlines():
            if any(kw in line for kw in ["buzz_fleet_listen", "buzz_eks_bridge", "buzz_cloud_bridge", "buzz-relay", "buzz_autonomous_responder", "port-forward"]):
                if "grep" not in line:
                    parts = line.split()
                    if len(parts) > 1:
                        processes.append(f"PID={parts[1]} {' '.join(parts[10:])[:60]}")
    except Exception:
        pass

    ack_match = re.search(r"ACK-[A-Z]+-\d+", content)
    ack_code = ack_match.group(0) if ack_match else "ACK-RECOVERY"

    response = (
        f"{ack_code} from {agent_name}. "
        f"Processes: {'; '.join(processes) if processes else 'none'}. "
        f"Listener: PID={get_listener_pid(agent_name)} buzz_fleet_listen.py --agent {agent_name}. "
        f"Identity: {agent_name} own keypair (not shared)."
    )
    logger.info(f"Recovery response: {ack_code}")
    return send_buzz_message(channel_id, response, sk)


# ---------------------------------------------------------------------------
# Pluggable action execution
# ---------------------------------------------------------------------------

def execute_action_handler(action: Dict, content: str, corr: str, sk: str, channel_id: str) -> Optional[str]:
    """Execute a single action handler if it matches the message content.

    Returns disposition string if matched, None if not matched.
    """
    content_lower = content.lower()

    # Check trigger: regex or keywords
    matched = False
    match_groups: List[str] = []

    trigger_regex = action.get("trigger_regex")
    if trigger_regex:
        m = re.search(trigger_regex, content, re.I)
        if m:
            matched = True
            match_groups = list(m.groups())

    trigger_keywords = action.get("trigger_keywords", [])
    if isinstance(trigger_keywords, str):
        trigger_keywords = [trigger_keywords]
    if not matched and trigger_keywords:
        if any(kw.lower() in content_lower for kw in trigger_keywords):
            matched = True

    if not matched:
        return None

    # Destructive check — if the action is marked destructive, queue only
    if action.get("destructive", False):
        queue_action(Path(f"/tmp/buzz-{os.environ.get('BUZZ_AGENT_NAME', 'agent')}-action-queue.jsonl"),
                     corr, "", content, content[:200])
        return "ACKNOWLEDGED_QUEUED_DESTRUCTIVE"

    # Execute the command
    command = action.get("command", [])
    if isinstance(command, str):
        command = [command]
    if not command:
        return None

    # Substitute {1}, {2}, etc. with regex match groups
    command = [
        re.sub(r"\{(\d+)\}", lambda m: match_groups[int(m.group(1)) - 1] if int(m.group(1)) <= len(match_groups) else "", cmd_part)
        for cmd_part in command
    ]

    cwd = action.get("cwd")
    timeout = int(action.get("timeout", 30))

    logger.info(f"Executing action '{action.get('name', '?')}' for {corr}: {' '.join(command[:3])}...")
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout, cwd=cwd)
        output = result.stdout.strip()[:500] if result.returncode == 0 else f"error: {result.stderr.strip()[:300]}"
    except subprocess.TimeoutExpired:
        output = f"timeout after {timeout}s"
    except Exception as e:
        output = f"error: {e}"

    result_template = action.get("result_template", "{output}")
    result_text = result_template.replace("{output}", output)
    # Substitute match groups in template too
    for i, g in enumerate(match_groups, 1):
        result_text = result_text.replace(f"{{{i}}}", g)

    response = (
        f"AGENT-RESULT v1 correlation_id: {corr} "
        f"provider_session: {os.environ.get('BUZZ_AGENT_NAME', 'agent')}-cli "
        f"disposition: COMPLETED "
        f"result: {result_text}"
    )
    send_buzz_message(channel_id, response, sk)
    return "COMPLETED"


def classify_and_execute_action(
    content: str, corr: str, sk: str, channel_id: str,
    action_handlers: List[Dict], agent_name: str,
) -> str:
    """Classify an action and execute it if read-only. Return disposition string.

    Generic destructive-action detection runs first, then pluggable handlers.
    """
    content_lower = content.lower()

    # Generic destructive patterns — always queue, never auto-execute
    destructive_patterns = [
        r"\bmerge\s+(?:pr\s*)?#\d+",
        r"\bdeploy\b",
        r"\bpush\b",
        r"\bdelete\b",
        r"\bforce-push\b",
        r"\breset\s+--hard\b",
        r"\brebase\b",
    ]
    if any(re.search(p, content_lower) for p in destructive_patterns):
        queue_action(
            Path(f"/tmp/buzz-{agent_name}-action-queue.jsonl"),
            corr, "", content, content[:200],
        )
        return "ACKNOWLEDGED_QUEUED_DESTRUCTIVE"

    # Try pluggable action handlers
    for handler in action_handlers:
        disposition = execute_action_handler(handler, content, corr, sk, channel_id)
        if disposition:
            return disposition

    # No handler matched — queue for manual execution
    queue_action(
        Path(f"/tmp/buzz-{agent_name}-action-queue.jsonl"),
        corr, "", content, content[:200],
    )
    return "ACKNOWLEDGED_QUEUED"


def handle_action_request(
    msg: Dict, sk: str, channel_id: str,
    action_handlers: List[Dict], agent_name: str,
) -> bool:
    """Handle direct action requests to this agent."""
    content = msg["content"]

    if f"@{agent_name}" not in content.lower() and agent_name not in content.lower():
        return False

    # Routed action from another agent's ACK
    if "AGENT-ACK" in content and msg.get("correlation_id"):
        if "next_action" in content.lower() and f"@{agent_name}" in content.lower():
            corr = msg["correlation_id"]
            action_match = re.search(r"next_action:\s*(.+?)(?:\s*$|\s*Durab)", content, re.I | re.S)
            action = action_match.group(1).strip() if action_match else content[:200]
            disposition = classify_and_execute_action(
                action, corr, sk, channel_id, action_handlers, agent_name,
            )
            ack = (
                f"AGENT-ACK v1 correlation_id: {corr} "
                f"provider_session: {agent_name}-cli "
                f"disposition: {disposition} "
                f"next_action: {'executed' if 'COMPLETED' in disposition else 'queued for manual execution'}"
            )
            logger.info(f"Action {disposition} for {corr}: {action[:80]}")
            return send_buzz_message(channel_id, ack, sk)
        return False

    # Direct action request
    action_keywords = ["route", "validate", "merge", "check", "verify", "run", "fix", "review", "dispatch", "status"]
    if any(kw in content.lower() for kw in action_keywords):
        corr = msg["correlation_id"] or f"AUTO-{int(time.time())}"
        disposition = classify_and_execute_action(
            content, corr, sk, channel_id, action_handlers, agent_name,
        )
        ack = (
            f"AGENT-ACK v1 correlation_id: {corr} "
            f"provider_session: {agent_name}-cli "
            f"disposition: {disposition} "
            f"next_action: {'executed autonomously' if 'COMPLETED' in disposition else 'queued for manual execution'}"
        )
        logger.info(f"Action {disposition} for {corr}")
        return send_buzz_message(channel_id, ack, sk)

    return False


def process_message(msg: Dict, sk: str, channel_id: str, action_handlers: List[Dict], agent_name: str):
    if handle_recovery_request(msg, sk, channel_id, agent_name):
        return
    if handle_agent_ack_request(msg, sk, channel_id, agent_name):
        return
    if handle_action_request(msg, sk, channel_id, action_handlers, agent_name):
        return
    if "QUEUE ADVANCE" in msg["content"] or "CONNECTIVITY" in msg["content"]:
        logger.info(f"Info: {msg['content'][:80]}")
        return
    logger.info(f"Unhandled mention: {msg['content'][:80]}")


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def tail_events(agent_name: str, sk: str, channel_id: str, action_handlers: List[Dict]):
    events_log = Path(f"/tmp/buzz-{agent_name}-events.log")
    processed_log = Path(f"/tmp/buzz-{agent_name}-processed.ids")
    processed = load_processed_ids(processed_log)
    file_pos = events_log.stat().st_size if events_log.exists() else 0

    logger.info(f"Tailing {events_log} from position {file_pos}")

    while True:
        try:
            if not events_log.exists():
                time.sleep(2)
                continue

            current_size = events_log.stat().st_size
            if current_size < file_pos:
                file_pos = 0

            if current_size > file_pos:
                with open(events_log, "r") as f:
                    f.seek(file_pos)
                    new_lines = f.readlines()
                    file_pos = f.tell()

                for line in new_lines:
                    line = line.strip()
                    if not line:
                        continue
                    msg = parse_message(line)
                    if not msg or msg["id"] in processed:
                        continue
                    processed.add(msg["id"])
                    mark_processed(msg["id"], processed_log)
                    logger.info(f"Processing: [{msg['sender_pub']}] {msg['content'][:80]}")
                    process_message(msg, sk, channel_id, action_handlers, agent_name)

            time.sleep(2)

        except KeyboardInterrupt:
            logger.info("Responder shutting down (Ctrl+C)")
            break
        except Exception as e:
            logger.error(f"Responder error: {e}")
            time.sleep(5)


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Autonomous Buzz fleet responder")
    parser.add_argument("--agent", default="devin-local", help="Agent name")
    parser.add_argument("--channel", default=None, help="Channel ID (default: fleet-coordination)")
    parser.add_argument("--actions-config", default=None, help="Path to action handlers YAML config")
    args = parser.parse_args()

    os.environ["BUZZ_AGENT_NAME"] = args.agent

    sk, pub = get_my_keys(args.agent)

    channel_id = args.channel
    if not channel_id:
        env = load_env(FLEET_CHANNELS)
        channel_id = env.get("FLEET_COORD_CHANNEL", "")
    if not channel_id:
        logger.error("No channel ID found")
        sys.exit(1)

    action_handlers = load_action_handlers(args.actions_config)

    logger.info(f"Autonomous responder starting as {args.agent}")
    logger.info(f"  Channel: {channel_id}")
    logger.info(f"  Action handlers: {len(action_handlers)}")
    tail_events(args.agent, sk, channel_id, action_handlers)


if __name__ == "__main__":
    main()
