#!/usr/bin/env python3
"""Seed and independently verify disposable local content-deletion fixtures.

Uses SQL fixtures with synthetic signatures; these are database handler tests,
not evidence that the relay accepted a signed client event. No service lifecycle
or deletion command is performed by this script.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlparse
import uuid


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def sql(statement):
    url = os.environ.get("DATABASE_URL", "")
    parsed = urlparse(url)
    if parsed.scheme not in ("postgres", "postgresql") or parsed.hostname not in (
        "localhost", "127.0.0.1", "::1"
    ) or not parsed.path.strip("/"):
        raise ValueError("DATABASE_URL must name a disposable database on localhost")
    result = subprocess.run(
        [os.environ.get("PSQL", "psql"), "-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-d", url],
        input="SET TIME ZONE 'UTC';\n" + statement,
        text=True, capture_output=True, check=False, timeout=120,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


def rows(statement):
    return json.loads(sql("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM (" + statement + ") t;"))


def ids(values):
    return ",".join("decode(" + literal(value) + ",'hex')" for value in values) or "NULL::bytea"


def snapshot(manifest):
    community = literal(manifest["community"])
    other = literal(manifest["other_community"])
    state = {}
    for group in ("deleted", "preserved"):
        event_ids = ids(manifest[group + "_events"])
        state[group + "_events"] = rows(f"SELECT * FROM events WHERE community_id={community} AND id IN ({event_ids})")
        state[group + "_reactions"] = rows(f"SELECT * FROM reactions WHERE community_id={community} AND reaction_event_id IN ({ids(manifest[group + '_reaction_events'])})")
        state[group + "_mentions"] = rows(f"SELECT * FROM event_mentions WHERE community_id={community} AND event_id IN ({ids(manifest[group + '_mentions'])})")
    state["other_events"] = rows(f"SELECT * FROM events WHERE community_id={other} AND id IN ({ids(manifest['other_events'])})")
    channel = literal(manifest["channel"])
    other_channel = literal(manifest["other_channel"])
    state["channels"] = rows(f"SELECT * FROM channels WHERE (community_id={community} AND id={channel}) OR (community_id={other} AND id={other_channel})")
    state["thread_metadata"] = rows(f"SELECT * FROM thread_metadata WHERE (community_id={community} AND channel_id={channel}) OR (community_id={other} AND channel_id={other_channel})")
    return state


def seed(args):
    path = Path(args.manifest)
    if path.exists():
        raise ValueError("manifest already exists; use a new path for a new additive fixture")
    for name in ("target", "survivor"):
        if not re.fullmatch(r"[0-9a-f]{64}", getattr(args, name) or ""):
            raise ValueError(f"--{name} must be a lowercase 64-character public key")
    if args.target == args.survivor:
        raise ValueError("target and survivor must differ")
    community = str(uuid.UUID(args.community or ""))
    other = str(uuid.UUID(args.other_community or ""))
    if community == other:
        raise ValueError("fixture requires two different communities")
    token = uuid.uuid4().hex
    event_id = lambda name: hashlib.sha256((token + ":" + name).encode()).hexdigest()
    roots = [event_id(f"root-{n}") for n in range(250)]
    edits = [event_id(f"edit-{n}") for n in range(10)]
    reactions = [event_id(f"reaction-{n}") for n in range(20)]
    replies = [event_id(f"reply-{n}") for n in range(5)]
    survivor_root = event_id("survivor-root")
    survivor_reaction = event_id("survivor-reaction")
    channel, other_channel = str(uuid.uuid4()), str(uuid.uuid4())
    manifest = {
        "version": 1, "fixture": "user-deletion-content", "target": args.target,
        "survivor": args.survivor, "community": community, "other_community": other,
        "channel": channel, "other_channel": other_channel,
        "deleted_events": roots + edits + reactions,
        "preserved_events": replies + [survivor_root, survivor_reaction],
        "deleted_reaction_events": reactions,
        "preserved_reaction_events": [survivor_reaction],
        "deleted_mentions": roots[:10], "preserved_mentions": [survivor_root],
        # Same id and timestamp in a different tenant is an intentional control.
        "other_events": [roots[-1]],
        "preserved_thread_ids": roots[:5] + replies + [survivor_root],
        "counts": {"target_messages": 250, "edits": 10, "reaction_events": 21,
                   "retained_replies": 5, "mentions": 11, "cross_community_controls": 1},
    }
    commands = ["BEGIN;"]
    for c, ch in ((community, channel), (other, other_channel)):
        commands.append(f"INSERT INTO channels(community_id,id,name,created_by) VALUES({literal(c)},{literal(ch)},{literal('deletion-content-' + ch[:8])},decode({literal(args.survivor)},'hex'));")
        for actor in (args.target, args.survivor):
            commands.append(f"INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES({literal(c)},{literal(ch)},decode({literal(actor)},'hex'));")
    records = []
    for n, key in enumerate(roots):
        records.append((community, channel, key, args.target, 9, [["h", channel]], f"Target fixture message {n}", n + 1))
    for n, key in enumerate(edits):
        records.append((community, channel, key, args.target, 40003, [["h", channel], ["e", roots[n]]], f"Edited fixture message {n}", 301 + n))
    for n, key in enumerate(reactions):
        records.append((community, channel, key, args.survivor, 7, [["h", channel], ["e", roots[n]]], "+", 401 + n))
    for n, key in enumerate(replies):
        records.append((community, channel, key, args.survivor, 9, [["h", channel], ["e", roots[n], "", "reply"]], f"Retained reply {n}", 501 + n))
    records.extend([
        (community, channel, survivor_root, args.survivor, 9, [["h", channel]], "Retained unrelated message", 601),
        (community, channel, survivor_reaction, args.survivor, 7, [["h", channel], ["e", survivor_root]], "+", 602),
        (other, other_channel, roots[-1], args.target, 9, [["h", other_channel]], "Other-community target control", 250),
    ])
    for c, ch, key, actor, kind, tags, content, offset in records:
        commands.append(f"INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig,channel_id) VALUES({literal(c)},decode({literal(key)},'hex'),decode({literal(actor)},'hex'),date_trunc('month',now())+make_interval(secs=>{offset}),{kind},{literal(json.dumps(tags))}::jsonb,{literal(content)},decode(repeat('00',64),'hex'),{literal(ch)});")
    for n, reaction in enumerate(reactions + [survivor_reaction]):
        root = roots[n] if n < len(reactions) else survivor_root
        commands.append(f"INSERT INTO reactions(community_id,event_created_at,event_id,pubkey,emoji,reaction_event_id) SELECT community_id,created_at,id,decode({literal(args.survivor)},'hex'),'+',decode({literal(reaction)},'hex') FROM events WHERE community_id={literal(community)} AND id=decode({literal(root)},'hex');")
    for root in roots[:10] + [survivor_root]:
        commands.append(f"INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,channel_id,event_kind) SELECT community_id,{literal(args.survivor)},id,created_at,channel_id,kind FROM events WHERE community_id={literal(community)} AND id=decode({literal(root)},'hex');")
    commands.append(f"INSERT INTO thread_metadata(community_id,event_created_at,event_id,channel_id) SELECT community_id,created_at,id,channel_id FROM events WHERE (community_id={literal(community)} AND id IN ({ids(roots + [survivor_root])})) OR (community_id={literal(other)} AND id=decode({literal(roots[-1])},'hex'));")
    for root, reply in zip(roots[:5], replies):
        commands.append(f"INSERT INTO thread_metadata(community_id,event_created_at,event_id,channel_id,parent_event_id,parent_event_created_at,root_event_id,root_event_created_at,depth) SELECT r.community_id,r.created_at,r.id,r.channel_id,p.id,p.created_at,p.id,p.created_at,1 FROM events r JOIN events p ON p.community_id=r.community_id WHERE r.community_id={literal(community)} AND r.id=decode({literal(reply)},'hex') AND p.id=decode({literal(root)},'hex');")
        commands.append(f"UPDATE thread_metadata SET reply_count=1,descendant_count=1,last_reply_at=(SELECT received_at FROM events WHERE community_id={literal(community)} AND id=decode({literal(reply)},'hex')) WHERE community_id={literal(community)} AND event_id=decode({literal(root)},'hex');")
    commands.append("COMMIT;")
    path.parent.mkdir(parents=True, exist_ok=True)
    # Reserve the manifest before touching SQL so an accidental rerun cannot seed
    # duplicate data under a discarded identity. A failed seed is visibly marked.
    with path.open("x") as output:
        json.dump({**manifest, "seed_complete": False}, output, indent=2, sort_keys=True)
    sql("\n".join(commands))
    manifest["before"] = snapshot(manifest)
    expected_counts = {"deleted_events": 280, "preserved_events": 7,
        "deleted_reactions": 20, "preserved_reactions": 1,
        "deleted_mentions": 10, "preserved_mentions": 1,
        "other_events": 1, "channels": 2, "thread_metadata": 257}
    if any(len(manifest["before"][name]) != count for name, count in expected_counts.items()):
        raise AssertionError("seed did not produce all expected fixture rows; manifest stays incomplete")
    manifest["seed_complete"] = True
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"record": "content_fixture_seeded", "manifest": str(path), "counts": manifest["counts"]}))


def verify(manifest, phase, completed, baseline):
    actual = snapshot(manifest)
    expected = baseline
    approved = {manifest["community"], manifest["other_community"]}
    if not set(completed).issubset(approved):
        raise ValueError("completed community is outside this fixture")
    if phase == "preview" and completed:
        raise ValueError("preview verification cannot have completed communities")
    if phase == "after-content":
        completed = set(completed or [manifest["community"]])
        expected = json.loads(json.dumps(expected))
        if manifest["community"] in completed:
            for key in ("deleted_events", "deleted_reactions", "deleted_mentions"):
                expected[key] = []
        if manifest["other_community"] in completed:
            expected["other_events"] = []
        kept = {"\\x" + value for value in manifest["preserved_thread_ids"]}
        expected["thread_metadata"] = [row for row in expected["thread_metadata"]
            if (row["community_id"] == manifest["community"] and
                (manifest["community"] not in completed or row["event_id"] in kept))
            or (row["community_id"] == manifest["other_community"] and
                manifest["other_community"] not in completed)]
    failures = [name for name in expected if actual[name] != expected[name]]
    if failures:
        raise AssertionError("content fixture mismatch: " + ", ".join(failures))
    print(json.dumps({"record": "content_fixture_verified", "phase": phase,
                      "counts": {name: len(value) for name, value in actual.items()}}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("seed", "snapshot", "verify"))
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--target")
    parser.add_argument("--survivor")
    parser.add_argument("--community")
    parser.add_argument("--other-community")
    parser.add_argument("--phase", choices=("preview", "after-content"), default="preview")
    parser.add_argument("--completed-community", action="append", default=[],
                        type=lambda value: str(uuid.UUID(value)),
                        help="Community whose content stage finished; repeat for both (default: primary)")
    args = parser.parse_args()
    if args.action == "seed":
        seed(args)
        return
    manifest_path = Path(args.manifest)
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    digest = hashlib.sha256(manifest_bytes).hexdigest()
    baseline_path = Path(str(manifest_path) + ".snapshot.json")
    if manifest.get("fixture") != "user-deletion-content" or manifest.get("version") != 1 or not manifest.get("seed_complete"):
        raise ValueError("manifest is not a successfully seeded content fixture")
    if args.action == "snapshot":
        observation = {"manifest_sha256": digest, "state": snapshot(manifest)}
        if baseline_path.exists():
            if json.loads(baseline_path.read_text()) != observation:
                raise ValueError("immutable content baseline differs; refusing to overwrite it")
        else:
            with baseline_path.open("x") as output:
                json.dump(observation, output, indent=2, sort_keys=True)
                output.write("\n")
        print(json.dumps({"record": "content_fixture_snapshot", "baseline": str(baseline_path),
                          "counts": {name: len(value) for name, value in observation["state"].items()}}))
    else:
        baseline = manifest["before"]
        if baseline_path.exists():
            saved = json.loads(baseline_path.read_text())
            if saved.get("manifest_sha256") != digest:
                raise ValueError("content baseline belongs to a different manifest")
            baseline = saved["state"]
        verify(manifest, args.phase, args.completed_community, baseline)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, AssertionError, OSError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
