#!/usr/bin/env python3
"""Add account cases to a disposable local DB and verify independent snapshots."""
import argparse
import copy
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit
import uuid


ACCOUNT_TABLES = (
    "api_tokens", "relay_invites", "channel_members", "join_policy_acceptances",
    "relay_members", "pubkey_allowlist", "personal_read_frontiers",
    "personal_read_accounts", "archived_identities", "parameterized_event_watermarks",
    "users",
)
CONTROL_TABLES = (
    "entity_deletion_requests", "entity_deletion_stage_history", "entity_deletion_approvals",
)


def check(value, message):
    if not value:
        raise RuntimeError(message)


def sql(query):
    # libpq receives the URL via the environment, not command arguments/logs.
    connection = os.environ["DATABASE_URL"]
    parsed = urlsplit(connection)
    check(parsed.scheme in ("postgres", "postgresql") and
          parsed.hostname in ("127.0.0.1", "localhost", "::1") and not parsed.query,
          "fixture requires a loopback DATABASE_URL without connection overrides")
    result = subprocess.run(
        [os.environ.get("PSQL", "psql"), "-XqAt", "-v", "ON_ERROR_STOP=1"],
        input=query, text=True, capture_output=True, timeout=60,
        env={**os.environ, "PGDATABASE": unquote(parsed.path.lstrip("/")),
             "PGHOST": parsed.hostname, "PGHOSTADDR": "::1" if parsed.hostname == "::1" else "127.0.0.1",
             "PGPORT": str(parsed.port or 5432), "PGUSER": unquote(parsed.username or ""),
             "PGPASSWORD": unquote(parsed.password or ""),
             "PGOPTIONS": "-c timezone=UTC -c statement_timeout=30000 -c lock_timeout=3000"},
    )
    check(result.returncode == 0, f"fixture SQL failed: {result.stderr}")
    return result.stdout.strip()


def snapshot():
    # A single read-only snapshot includes workflow state, so preview cannot
    # silently advance a stage or record a deletion receipt.
    selections = [f"'{t}', (SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)), '[]'::jsonb) FROM {t} r)"
                  for t in ACCOUNT_TABLES + CONTROL_TABLES]
    return json.loads(sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
                          "SELECT jsonb_build_object(" + ",".join(selections) + ");\nCOMMIT;"))


def seed(args):
    check(not args.manifest.exists(), "manifest already exists; use a fresh fixture manifest")
    for key in (args.target, args.survivor):
        check(key is not None and re.fullmatch(r"[0-9a-f]{64}", key), "keys must be lowercase hex")
    check(args.target != args.survivor, "target and survivor must differ")
    community, other = str(uuid.UUID(args.community)), str(uuid.UUID(args.other_community))
    check(community != other, "communities must differ")
    channel = str(uuid.uuid4())
    statements = ["BEGIN;"]
    for cid in (community, other):
        statements.append(f"INSERT INTO communities(id,host) VALUES('{cid}','{cid}.example.org') ON CONFLICT(id) DO NOTHING;")
        statements.append(f"INSERT INTO channels(community_id,id,name,created_by) VALUES('{cid}','{channel}','account-fixture',decode('{args.survivor}','hex'));")
        for key in (args.target, args.survivor):
            token = str(uuid.uuid4())
            statements.append(f"""
INSERT INTO users(community_id,pubkey) VALUES('{cid}',decode('{key}','hex')) ON CONFLICT DO NOTHING;
INSERT INTO relay_members(community_id,pubkey,role,added_by) VALUES('{cid}','{key}','member','{args.target}') ON CONFLICT DO NOTHING;
INSERT INTO personal_read_accounts(community_id,actor) VALUES('{cid}',decode('{key}','hex')) ON CONFLICT DO NOTHING;
INSERT INTO personal_read_frontiers(community_id,actor,channel_id,root_id,through_timestamp)
 SELECT '{cid}',decode('{key}','hex'),'{channel}',decode(lpad(to_hex(n),64,'0'),'hex'),now() FROM generate_series(1,250)n;
INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes,revoked_by,revoked_at)
 VALUES('{cid}','{token}',decode('{key}','hex'),decode(md5('{token}')||md5('{token}'),'hex'),'fixture revoked token','[]',decode('{args.target}','hex'),now());
INSERT INTO channel_members(community_id,channel_id,pubkey,invited_by,removed_by,removed_at)
 VALUES('{cid}','{channel}',decode('{key}','hex'),decode('{args.target}','hex'),decode('{args.target}','hex'),now());
INSERT INTO pubkey_allowlist(community_id,pubkey,added_by) VALUES('{cid}',decode('{key}','hex'),decode('{args.target}','hex')) ON CONFLICT DO NOTHING;
INSERT INTO archived_identities(community_id,pubkey,actor,consent_path,replaced_by,request_event_id)
 VALUES('{cid}','{key}','{key}','self','{args.target}','account-fixture') ON CONFLICT DO NOTHING;
INSERT INTO join_policy_acceptances(community_id,pubkey,policy_version)
 VALUES('{cid}','{key}',repeat('a',64)) ON CONFLICT DO NOTHING;
""")
        statements.append(f"""
INSERT INTO relay_invites(community_id,created_by,token_hash,expires_at)
 VALUES('{cid}','{args.target}',decode(md5('{channel}')||md5('{channel}'),'hex'),now());
INSERT INTO parameterized_event_watermarks(community_id,pubkey,kind,d_tag,created_at,event_id)
 VALUES('{cid}',decode('{args.target}','hex'),30078,'account-fixture',now(),decode('{args.target}','hex'));
""")
    statements.append("COMMIT;")
    sql("\n".join(statements))
    manifest = {"version": 1, "target": args.target, "survivor": args.survivor,
                "community": community, "other_community": other, "channel": channel}
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    with args.manifest.open("x") as output:
        json.dump(manifest, output, indent=2)
    return {"seeded": True, "manifest": str(args.manifest), "frontiers_per_identity_per_community": 250}


def expected_rows(table, rows, target, communities):
    # This independent oracle describes retained data, not production selectors.
    owner = {"api_tokens": "owner_pubkey", "relay_invites": "created_by",
             "channel_members": "pubkey", "join_policy_acceptances": "pubkey",
             "relay_members": "pubkey", "pubkey_allowlist": "pubkey",
             "personal_read_frontiers": "actor", "personal_read_accounts": "actor"}
    attribution = {"api_tokens": ("revoked_by",), "channel_members": ("invited_by", "removed_by"),
                   "relay_members": ("added_by",), "pubkey_allowlist": ("added_by",),
                   "archived_identities": ("replaced_by",)}
    matches = (target, "\\x" + target)
    result = []
    for original in rows:
        row = copy.deepcopy(original)
        if row["community_id"] in communities:
            if table in owner and row[owner[table]] in matches:
                continue
            changed = False
            for field in attribution.get(table, ()):
                if row[field] in matches:
                    row[field] = None
                    changed = True
            if changed and table in ("channel_members", "relay_members"):
                row.pop("sync_modified_at" if table == "channel_members" else "updated_at")
        result.append(row)
    return result


def verify(manifest, phase, completed):
    before = manifest.get("before")
    check(before is not None, "take snapshot after all seeders and request preparation")
    after = snapshot()
    if phase == "preview":
        check(before == after, "preview changed account or deletion-control rows")
    else:
        communities = set(completed or [manifest["community"]])
        check(communities <= {manifest["community"], manifest["other_community"]},
              "completed community is outside this fixture")
        for table in ACCOUNT_TABLES:
            wanted = expected_rows(table, before[table], manifest["target"], communities)
            actual = copy.deepcopy(after[table])
            # Permit only the two documented synchronization timestamp changes.
            if table in ("channel_members", "relay_members"):
                timestamp = "sync_modified_at" if table == "channel_members" else "updated_at"
                key_fields = ("community_id", "channel_id", "pubkey") if table == "channel_members" else ("community_id", "pubkey")
                identity = lambda r: tuple(r[k] for k in key_fields)
                expected_map = {identity(r): r for r in wanted}
                before_map = {identity(r): r for r in before[table]}
                for row in actual:
                    key = identity(row)
                    if key in expected_map and timestamp not in expected_map[key]:
                        check(row[timestamp] >= before_map[key][timestamp], f"{table} sync timestamp regressed")
                        row.pop(timestamp)
            order = lambda r: json.dumps(r, sort_keys=True)
            check(sorted(wanted, key=order) == sorted(actual, key=order), f"unexpected changes or remaining work in {table}")
    return {"verified": phase, "account_erasure_complete": False,
            "scope": "account fixture tables; media/content verified separately"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seed", "snapshot", "verify"))
    parser.add_argument("--manifest", type=Path, required=True)
    for flag in ("target", "survivor", "community", "other-community"):
        parser.add_argument("--" + flag)
    parser.add_argument("--phase", choices=("preview", "after-account"), default="preview")
    parser.add_argument("--completed-community", type=lambda v: str(uuid.UUID(v)), action="append",
                        help="repeat for each account stage executed since snapshot")
    args = parser.parse_args()
    if args.command == "seed":
        result = seed(args)
    else:
        manifest = json.loads(args.manifest.read_text())
        check(manifest.get("version") == 1, "unsupported fixture manifest")
        if args.command == "snapshot":
            check("before" not in manifest, "baseline exists; use a new manifest for a new run")
            manifest["before"] = snapshot()
            args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")
            result = {"snapshot": str(args.manifest)}
        else:
            result = verify(manifest, args.phase, args.completed_community)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
