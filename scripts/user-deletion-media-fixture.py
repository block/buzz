#!/usr/bin/env python3
"""Add and verify synthetic media deletion fixtures in disposable local services."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.parse
import uuid


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def local_url(name, schemes):
    value = os.environ.get(name, "")
    parsed = urllib.parse.urlparse(value)
    require(parsed.scheme in schemes, f"{name} must be a URL")
    require(parsed.hostname in {"127.0.0.1", "localhost", "::1"},
            f"{name} must use a literal loopback host")
    require(not parsed.query and not parsed.fragment,
            f"{name} must not contain connection overrides or fragments")
    return value


class Services:
    def __init__(self):
        self.database = local_url("DATABASE_URL", {"postgres", "postgresql"})
        self.endpoint = local_url("BUZZ_S3_ENDPOINT", {"http", "https"})
        self.bucket = os.environ.get("BUZZ_S3_BUCKET", "")
        require(bool(re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", self.bucket)),
                "BUZZ_S3_BUCKET must name an existing local fixture bucket")
        self.env = os.environ.copy()
        database = urllib.parse.urlparse(self.database)
        require(database.username and database.path.strip("/"),
                "DATABASE_URL must include an explicit user and database")
        self.database_identity = {"host": database.hostname, "port": database.port or 5432,
                                  "database": urllib.parse.unquote(database.path.lstrip("/"))}
        for name in ("PGHOSTADDR", "PGSERVICE", "PGSERVICEFILE"):
            self.env.pop(name, None)
        self.env.update({"PGHOST": database.hostname, "PGPORT": str(database.port or 5432),
                         "PGUSER": urllib.parse.unquote(database.username),
                         "PGPASSWORD": urllib.parse.unquote(database.password or ""),
                         "PGDATABASE": urllib.parse.unquote(database.path.lstrip("/")),
                         "PGCONNECT_TIMEOUT": "5",
                         "PGOPTIONS": "-c timezone=UTC -c statement_timeout=30000",
                         "AWS_EC2_METADATA_DISABLED": "true", "AWS_PAGER": ""})
        for name in ("AWS_PROFILE", "AWS_DEFAULT_PROFILE", "AWS_SESSION_TOKEN"):
            self.env.pop(name, None)
        for target, source in (("AWS_ACCESS_KEY_ID", "BUZZ_S3_ACCESS_KEY"),
                               ("AWS_SECRET_ACCESS_KEY", "BUZZ_S3_SECRET_KEY")):
            require(bool(os.environ.get(source)), f"{source} is required for local fixtures")
            self.env[target] = os.environ[source]

    def run(self, args, text=None):
        result = subprocess.run(args, input=text, text=True, capture_output=True,
                                env=self.env, timeout=90, check=False)
        require(result.returncode == 0,
                f"{Path(args[0]).name} failed: {result.stderr.strip()}")
        return result.stdout.strip()

    def sql(self, query):
        return self.run([os.environ.get("PSQL", "psql"), "-X", "-qAt",
                         "-v", "ON_ERROR_STOP=1"], query)

    def s3(self, operation, *args):
        result = self.run([os.environ.get("AWS", "aws"), "--endpoint-url", self.endpoint,
                           "--region", os.environ.get("BUZZ_S3_REGION", "us-east-1"),
                           "--output", "json", "s3api", operation,
                           "--bucket", self.bucket, *args])
        return json.loads(result) if result else {}

    def put(self, key, data):
        with tempfile.NamedTemporaryFile() as file:
            file.write(data)
            file.flush()
            output = self.s3("put-object", "--key", key, "--body", file.name)
        version = output.get("VersionId")
        require(version and version != "null", "fixture bucket must have versioning enabled")
        return {"key": key, "version_id": version,
                "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def db_snapshot(services, manifest):
    hashes = ",".join(literal(case["hash"]) for case in manifest["cases"])
    ids = ",".join("decode(" + literal(event["id"]) + ",'hex')"
                   for event in manifest["events"])
    result = {}
    for table, where in (("media_uploaders", f"sha256 IN ({hashes})"),
                         ("media_publications", f"sha256 IN ({hashes})"),
                         ("events", f"id IN ({ids})")):
        rows = services.sql(f"SELECT COALESCE(jsonb_agg(row), '[]'::jsonb) "
                            f"FROM (SELECT to_jsonb(t) AS row FROM {table} t "
                            f"WHERE {where} ORDER BY to_jsonb(t)::text) selected;")
        result[table] = json.loads(rows)
    return result


def storage_snapshot(services, manifest):
    keys = {obj["key"] for case in manifest["cases"] for obj in case["objects"]}
    result = []
    # AWS CLI performs ListObjectVersions pagination; include delete markers so a
    # current-version-only delete cannot masquerade as permanent deletion.
    listing = services.s3("list-object-versions")
    for field in ("Versions", "DeleteMarkers"):
        for obj in listing.get(field, []):
            if obj["Key"] in keys:
                result.append({"key": obj["Key"], "version_id": obj["VersionId"],
                               "kind": field, "etag": obj.get("ETag"),
                               "size": obj.get("Size"), "latest": obj["IsLatest"]})
    return sorted(result, key=canonical)


def snapshot(services, manifest):
    return {"database": db_snapshot(services, manifest),
            "storage": storage_snapshot(services, manifest)}


def seed(services, args):
    require(not args.manifest.exists(), "manifest already exists; use a new fixture/manifest")
    for key in (args.target, args.survivor):
        require(key and re.fullmatch(r"[0-9a-f]{64}", key), "target/survivor must be hex pubkeys")
    require(args.target != args.survivor, "target and survivor must differ")
    community = str(uuid.UUID(args.community))
    other = str(uuid.UUID(args.other_community))
    require(community != other, "community controls must differ")
    require(services.s3("get-bucket-versioning").get("Status") == "Enabled",
            "enable versioning on the disposable bucket before seeding")
    found = services.sql("SELECT count(*) FROM communities WHERE id IN ("
                         + literal(community) + "," + literal(other) + ");")
    require(found == "2", "harness must create both communities before media seed")
    manifest = {"version": 1, "seed_complete": False, "target": args.target,
                "survivor": args.survivor, "community": community,
                "other_community": other, "endpoint": services.endpoint,
                "bucket": services.bucket, "database": services.database_identity,
                "cases": [], "events": []}
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    # Reserve the manifest before mutating either service; interrupted seeds stay
    # visibly incomplete and are never mistaken for successful fixtures.
    with args.manifest.open("x") as file:
        json.dump(manifest, file, indent=2)
    statements = ["BEGIN;", "SET LOCAL TIME ZONE 'UTC';"]
    cases = [("exclusive", True), ("shared-user", False), ("shared-community", False),
             ("other-metadata", False), ("unrelated", False), ("unposted", True)]
    for name, delete in cases:
        payload = f"local deletion fixture {name} {uuid.uuid4()}\n".encode()
        sha = hashlib.sha256(payload).hexdigest()
        case = {"name": name, "hash": sha, "delete": delete, "objects": []}
        manifest["cases"].append(case)
        case["objects"].append(services.put(f"{sha}.txt", payload))
        if name == "exclusive":
            case["objects"].append(services.put(f"{sha}.txt", payload))
            case["objects"].append(services.put(f"{sha}.thumb.webp", b"synthetic-thumbnail"))
        metadata_community = other if name == "other-metadata" else community
        case["objects"].append(services.put(
            f"_meta/{metadata_community}/{sha}.json", canonical({"fixture": name}).encode()))
        actor = args.survivor if name == "unrelated" else args.target
        upload_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        record = {"version": 1, "community_id": community, "sha256": sha,
                  "event_id": upload_id, "uploader_id": actor}
        case["objects"].append(services.put(
            f"_uploads/{community}/{sha}/{upload_id}.json", canonical(record).encode()))
        claims = [] if name == "unposted" else [(community, actor)]
        if name == "shared-user":
            claims.append((community, args.survivor))
        if name == "shared-community":
            claims.append((other, args.survivor))
        for claim_community, pubkey in claims:
            statements.append("INSERT INTO media_uploaders(community_id,sha256,pubkey) VALUES ("
                              f"{literal(claim_community)},{literal(sha)},decode({literal(pubkey)},'hex'));")
            statements.append("INSERT INTO media_publications(community_id,sha256) VALUES ("
                              f"{literal(claim_community)},{literal(sha)}) ON CONFLICT DO NOTHING;")
        for claim_community, pubkey in claims:
            event_id = hashlib.sha256(uuid.uuid4().bytes).hexdigest()
            manifest["events"].append({"id": event_id, "community": claim_community,
                                        "pubkey": pubkey, "hash": sha})
            content = f"Synthetic media fixture {name}: /media/{sha}.txt"
            statements.append("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) VALUES ("
                              f"{literal(claim_community)},decode({literal(event_id)},'hex'),"
                              f"decode({literal(pubkey)},'hex'),date_trunc('second',now()),9,'[]',"
                              f"{literal(content)},decode(repeat('00',64),'hex'));")
    statements.append("COMMIT;")
    services.sql("\n".join(statements))
    manifest["baseline"] = snapshot(services, manifest)
    manifest["seed_complete"] = True
    args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")
    return {"seeded": True, "cases": len(manifest["cases"]),
            "events": len(manifest["events"]), "manifest": str(args.manifest)}


def verify(services, manifest, phase):
    actual = snapshot(services, manifest)
    expected = json.loads(json.dumps(manifest["baseline"]))
    if phase in {"after-media", "after-content"}:
        deleted_keys = {obj["key"] for case in manifest["cases"] if case["delete"]
                        for obj in case["objects"]}
        expected["storage"] = [obj for obj in expected["storage"]
                               if obj["key"] not in deleted_keys]
        target_bytes = "\\x" + manifest["target"]
        expected["database"]["media_uploaders"] = [
            row for row in expected["database"]["media_uploaders"]
            if row["pubkey"] != target_bytes]
        if phase == "after-content":
            expected["database"]["events"] = [
                row for row in expected["database"]["events"] if row["pubkey"] != target_bytes]
    errors = []
    for name in ("storage", "database"):
        if actual[name] != expected[name]:
            errors.append({"surface": name, "expected": expected[name], "actual": actual[name]})
    require(not errors, "fixture verification failed: " + json.dumps(errors, sort_keys=True))
    return {"verified": True, "phase": phase, "cases": len(manifest["cases"]),
            "storage_versions": len(actual["storage"]),
            "database_rows": {name: len(rows) for name, rows in actual["database"].items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["seed", "snapshot", "verify"])
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--phase", choices=["before", "preview", "after-media", "after-content"],
                        default="before")
    for name in ("target", "survivor", "community", "other-community"):
        parser.add_argument("--" + name)
    args = parser.parse_args()
    services = Services()
    if args.command == "seed":
        require(args.community and args.other_community, "seed requires community inputs")
        output = seed(services, args)
    else:
        manifest = json.loads(args.manifest.read_text())
        require(manifest.get("seed_complete") is True, "fixture seed is incomplete")
        require(manifest["endpoint"] == services.endpoint and manifest["bucket"] == services.bucket,
                "fixture storage endpoint/bucket changed")
        require(manifest["database"] == services.database_identity,
                "fixture database identity changed")
        output = snapshot(services, manifest) if args.command == "snapshot" else verify(
            services, manifest, args.phase)
    print(json.dumps(output, indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as error:
        raise SystemExit(str(error)) from error
