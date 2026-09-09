#!/usr/bin/env python3
"""Assign a canvas-backed task and wake it using the dedicated Monitor identity.

Personal prototype: requires the installed buzz-task-channels skill and Buzz CLI.
Input is one JSON object on stdin. No credentials are accepted as arguments.
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys


def load_helper():
    path = Path.home() / ".agents/skills/buzz-task-channels/scripts/create-task-channel.py"
    if not path.is_file():
        raise RuntimeError("Install the buzz-task-channels skill before assigning tasks")
    spec = importlib.util.spec_from_file_location("task_channel_helper", path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    return helper


def assign(request, helper, state_dir):
    channel = request["channel_id"]
    pubkey = request["assignee_pubkey"]
    operation = request["operation_id"]
    if not helper.UUID.fullmatch(channel) or not helper.UUID.fullmatch(operation) or not helper.HEX64.fullmatch(pubkey):
        raise ValueError("Invalid assignment identifiers")
    args = argparse.Namespace(buzz_bin=os.environ.get("BUZZ_BIN", "buzz"))
    scope = hashlib.sha256((os.environ.get("BUZZ_RELAY_URL", "") + channel + operation).encode()).hexdigest()
    state_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    journal = state_dir / f"{scope}.json"
    with (state_dir / f"{channel}.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        state = json.loads(journal.read_text()) if journal.exists() else {"pubkey": pubkey}
        if state["pubkey"] != pubkey:
            raise ValueError("Assignment operation belongs to a different agent")

        def save():
            helper.atomic_write_json(journal, state)

        def canvas():
            return helper.run_buzz(args, ["canvas", "get", "--channel", channel])

        def set_canvas(content):
            helper.accepted_event(helper.run_buzz(args, ["canvas", "set", "--channel", channel, "--content", "-"], content=content), "canvas assignment")

        def same(left, right):
            return isinstance(left, str) and left.rstrip() == right.rstrip()

        current = canvas()
        if same(current, request["completed_canvas"]):
            return {"assigned": True, "notified": True}
        if not same(current, request["canvas_content"]):
            if not same(current, request["expected_canvas"]):
                raise RuntimeError("Canvas changed; refresh before assigning")
            set_canvas(request["canvas_content"])
        try:
            # Dedicated credential only: never inherit the human sender for a wake.
            environment = helper.monitor_environment(helper.monitor_private_key())
            monitor = helper.current_identity_pubkey(args, env=environment)
            creator = helper.current_identity_pubkey(args)
            if monitor == creator or monitor == pubkey:
                raise RuntimeError("Monitor must be a separate dedicated identity")
            members = helper.run_buzz(args, ["channels", "members", "--channel", channel])
            if not isinstance(members, list):
                raise RuntimeError("Could not read channel membership")
            present = {member.get("pubkey") for member in members if isinstance(member, dict)}
            for member, role in ((pubkey, "member"), (monitor, "bot")):
                step = f"member:{member}"
                if member not in present and member != creator:
                    helper.accepted_event(helper.run_buzz(args, ["channels", "add-member", "--channel", channel, "--pubkey", member, "--role", role]), "assignment membership")
                    state[step] = True
                    save()
            if not state.get("notified"):
                # A persisted in-flight marker avoids blindly duplicating an uncertain send.
                if state.get("sending"):
                    raise RuntimeError("Previous notification outcome is uncertain. Check channel history before retrying; the local assignment journal retains this attempt.")
                state["sending"] = True
                save()
                message = "You’ve been assigned this task. Start work here. Read the channel canvas and its experiment instructions; do not create a native Buzz task."
                receipt = helper.run_buzz(args, ["messages", "send", "--channel", channel, "--content", "-", "--mention", pubkey], content=message, env=environment)
                if not isinstance(receipt, dict) or receipt.get("accepted") is not True:
                    state["sending"] = False
                    save()
                    helper.accepted_event(receipt, "assignment notification")
                state["notified"] = True
                save()
            # Never overwrite edits made while membership/notification was in flight.
            if not same(canvas(), request["canvas_content"]):
                raise RuntimeError("Agent notified, but canvas changed before delivery status could be saved; retry to reconcile")
            set_canvas(request["completed_canvas"])
            return {"assigned": True, "notified": True}
        except Exception as error:
            return {"assigned": True, "notified": False, "error": str(error)}


def main():
    try:
        input_path = os.environ.get("BUZZ_TASK_ASSIGNMENT_INPUT")
        if input_path:
            with open(input_path) as source:
                request = json.load(source)
        else:
            request = json.load(sys.stdin)
        result = assign(request, load_helper(), Path.home() / ".local/state/buzz-task-assignments")
        print(json.dumps(result))
    except Exception as error:
        print(json.dumps({"assigned": False, "notified": False, "error": str(error)}))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
