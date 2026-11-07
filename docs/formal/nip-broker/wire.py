"""Executable NIP-BA wire examples; independent spec oracle, NOT production SDK.

Intentionally no networking or cryptographic signature verification. Validates
normalized request/result shapes and basic correlation. See test_wire.py.
"""
import json
import re
import uuid

HEX = re.compile(r"[0-9a-f]{64}\Z")
SLUG = re.compile(r"(?:core|mem/[a-z0-9][a-z0-9_-]{0,63}(?:/[a-z0-9][a-z0-9_-]{0,63})*)\Z")
WHITE_SPACE = "\u0009\u000a\u000b\u000c\u000d\u0020\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000"


def require(condition):
    if not condition:
        raise ValueError("invalid wire value")


def obj(value, required, optional=()):
    require(type(value) is dict and set(required) <= value.keys()
            and value.keys() <= set(required) | set(optional))


def integer(value, bits=32):
    require(type(value) is int and 0 <= value < 2 ** bits)


def string(value):
    require(type(value) is str)
    value.encode("utf-8")  # also reject lone surrogates


def scalar(value, maximum):
    string(value)
    require(bool(value.strip(WHITE_SPACE)) and len(value.strip(WHITE_SPACE)) <= maximum)


def opaque(value, maximum):
    string(value)
    require(1 <= len(value) <= maximum and all(0x21 <= ord(c) <= 0x7e for c in value))


def hex64(value, key=False):
    string(value)
    require(bool(HEX.fullmatch(value)))
    if key:
        x = int(value, 16)
        p = 2 ** 256 - 2 ** 32 - 977
        y2 = (x ** 3 + 7) % p
        require(x < p and pow(y2, (p - 1) // 2, p) == 1)


def channel(value):
    string(value)
    require(str(uuid.UUID(value)) == value)


def payload(value, maximum):
    scalar(value, maximum)
    require(len(value.encode("utf-8")) <= maximum)


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def parse(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result)
            result[key] = value
        return result

    def reject(_):
        raise ValueError("non-integer number")

    if isinstance(raw, bytes):
        raw = raw.decode("utf-8")
    value = json.loads(raw, object_pairs_hook=pairs, parse_float=reject, parse_constant=reject)

    def walk(v):
        require(v is not None)
        if type(v) is dict:
            for k, child in v.items():
                string(k)
                walk(child)
        elif type(v) is list:
            for child in v:
                walk(child)
        elif type(v) is str:
            string(v)
    walk(value)
    return value


SCHEMAS = {
    "channel.read": ("channelId", "rootEventId mentionsOnly cursor limit"),
    "message.post": ("channelId content", "mentions"),
    "message.reply": ("channelId content replyToEventId", "mentions"),
    "reaction.add": ("channelId targetEventId reaction", ""),
    "profile.set": ("", "displayName about picture"),
    "storage.address": ("slug", ""),
    "storage.get": ("slug", ""),
    "storage.put": ("slug value", ""),
    "presence.set": ("status", ""),
    "typing.set": ("channelId", ""),
    "observer.emit": ("frames", ""),
    "liveness.ping": ("channelId turnId", ""),
    "agents.create": ("channelId displayName systemPrompt", "runtime provider model respondTo"),
    "agents.update": ("target", "displayName systemPrompt runtime provider model respondTo"),
    "agents.delete": ("target", ""),
}
MUTABLE = set("displayName systemPrompt runtime provider model respondTo".split())
PUBLISHED = {"message.post": 9, "message.reply": 9, "reaction.add": 7,
             "profile.set": 0, "storage.put": 30174, "presence.set": 20001,
             "typing.set": 20002, "liveness.ping": 24200}
CODES = set("invalid_request unsupported_protocol_version unknown_action unsupported_action_version unsupported unauthenticated unauthorized request_id_conflict action_failed outcome_unknown internal".split())


def request(raw):
    r = parse(raw)
    obj(r, "type protocolVersion requestId actionVersion action args".split())
    require(r["type"] == "broker_request")
    opaque(r["requestId"], 128)
    for field in ("protocolVersion", "actionVersion"):
        integer(r[field], 16)
        require(r[field] == 1)
    require(type(r["action"]) is str and r["action"] in SCHEMAS)
    action, a = r["action"], r["args"]
    required, optional = SCHEMAS[action]
    obj(a, required.split(), optional.split())
    # Normalize only argument scalars. Keep raw body bytes outside this oracle
    # for retry hashing; never apply this to signed events or opaque content.
    def normalize(k, v):
        if k in ("content", "value", "payload", "cursor") or type(v) is not str:
            return v
        v = v.strip(WHITE_SPACE)
        if k in ("pubkey", "rootEventId", "replyToEventId", "targetEventId"):
            return v.lower()
        if k == "channelId":
            h = r"[0-9a-fA-F]"
            u = f"{h}{{8}}-{h}{{4}}-{h}{{4}}-{h}{{4}}-{h}{{12}}"
            require(bool(re.fullmatch(f"(?:{u}|{h}{{32}}|\\{{{u}\\}}|urn:uuid:{u})", v)))
            return str(uuid.UUID(v))
        return v
    for k, v in a.items():
        if k == "target" and type(v) is dict:
            a[k] = {f: normalize(f, item) for f, item in v.items()}
        elif k == "mentions" and type(v) is list:
            a[k] = [normalize("pubkey", item) for item in v]
        elif k == "frames" and type(v) is list:
            a[k] = [{f: normalize(f, item) for f, item in frame.items()}
                    if type(frame) is dict else frame for frame in v]
        else:
            a[k] = normalize(k, v)
    for k, v in a.items():
        if k == "channelId":
            channel(v)
        elif k in ("rootEventId", "replyToEventId", "targetEventId"):
            hex64(v)
        elif k in ("displayName", "systemPrompt", "about", "picture", "runtime", "provider", "model", "turnId", "reaction"):
            scalar(v, {"displayName": 120, "systemPrompt": 20000, "about": 2000, "reaction": 66}.get(k, 300))
        elif k == "respondTo":
            require(v in ("owner-only", "anyone"))
        elif k == "status":
            require(v in ("online", "away", "offline"))
        elif k == "mentionsOnly":
            require(type(v) is bool)
        elif k == "limit":
            integer(v)
            require(1 <= v <= 500)
        elif k == "cursor":
            opaque(v, 256)
        elif k == "content":
            payload(v, 65536)
        elif k == "mentions":
            require(type(v) is list and 1 <= len(v) <= 50)
            for pubkey in v:
                hex64(pubkey, True)
        elif k == "slug":
            string(v)
            require(len(v) <= 255 and bool(SLUG.fullmatch(v)))
        elif k == "target":
            require(type(v) is dict and set(v) in ({"name"}, {"pubkey"}))
            if "name" in v:
                scalar(v["name"], 120)
            else:
                hex64(v["pubkey"], True)
        elif k == "value":
            payload(v, 65535)
        elif k == "frames":
            require(type(v) is list and 1 <= len(v) <= 256)
            for frame in v:
                obj(frame, ("kind", "payload"))
                scalar(frame["kind"], 300)
                payload(frame["payload"], 65535)
            require(len(compact(a)) <= 65535)
    if action == "profile.set":
        require(bool(a))
    if action == "agents.update":
        require(bool(set(a) & MUTABLE))
    if action == "storage.put":
        body = {"slug": a["slug"], "profile" if a["slug"] == "core" else "value": a["value"]}
        require(len(compact(body)) <= 65535)
    return r


def result(raw, req):
    r = parse(raw)
    common = "type protocolVersion requestId status".split()
    require(type(r) is dict and r.get("status") in ("succeeded", "failed", "indeterminate"))
    succeeded = r["status"] == "succeeded"
    obj(r, common + (["action", "outcome"] if succeeded else ["error"]), ["replayed"])
    require(r["type"] == "broker_result")
    integer(r["protocolVersion"], 16)
    require(r["protocolVersion"] == 1 and r["requestId"] == req["requestId"])
    if "replayed" in r:
        require(type(r["replayed"]) is bool)
    if not succeeded:
        e = r["error"]
        obj(e, ("code", "message"))
        require(type(e["code"]) is str and e["code"] in CODES)
        string(e["message"])
        require((r["status"] == "indeterminate" and e["code"] in ("outcome_unknown", "internal"))
                or (r["status"] == "failed" and e["code"] != "outcome_unknown"))
        return r
    action, o, a = r["action"], r["outcome"], req["args"]
    require(action == req["action"])
    if action in PUBLISHED:
        obj(o, ("eventId", "kind", "createdAt"))
        hex64(o["eventId"])
        integer(o["kind"])
        integer(o["createdAt"], 64)
        require(o["kind"] == PUBLISHED[action])
    elif action == "channel.read":
        obj(o, ("messages",), ("nextCursor",))
        require(type(o["messages"]) is list and len(o["messages"]) <= a.get("limit", 100))
        if "nextCursor" in o:
            opaque(o["nextCursor"], 256)
        for event in o["messages"]:
            obj(event, "id pubkey created_at kind tags content sig".split())
            hex64(event["id"])
            hex64(event["pubkey"], True)
            integer(event["created_at"], 64)
            integer(event["kind"])
            string(event["content"])
            string(event["sig"])
            require(bool(re.fullmatch(r"[0-9a-f]{128}", event["sig"])))
            require(type(event["tags"]) is list)
            for tag in event["tags"]:
                require(type(tag) is list)
                for value in tag:
                    string(value)
    elif action == "storage.get":
        obj(o, (), ("value",))
        if "value" in o:
            string(o["value"])
    elif action == "storage.address":
        obj(o, ("authorPubkey", "kind", "dTag"))
        hex64(o["authorPubkey"], True)
        hex64(o["dTag"])
        integer(o["kind"])
        require(o["kind"] == 30174)
    elif action == "observer.emit":
        obj(o, ("accepted",))
        integer(o["accepted"])
        require(o["accepted"] <= len(a["frames"]))
    else:
        fields = ["agentPubkey", "displayName"]
        if action == "agents.create":
            fields.append("channelId")
        if action == "agents.update":
            fields.append("updatedFields")
        obj(o, fields)
        hex64(o["agentPubkey"], True)
        scalar(o["displayName"], 120)
        if action == "agents.create":
            channel(o["channelId"])
            require(uuid.UUID(o["channelId"]) == uuid.UUID(a["channelId"]))
        elif "pubkey" in a["target"]:
            require(o["agentPubkey"].lower() == a["target"]["pubkey"].lower())
        if action == "agents.update":
            changed = o["updatedFields"]
            require(type(changed) is list and all(type(v) is str for v in changed))
            require(changed == sorted(set(changed)) and set(changed) <= set(a) & MUTABLE)
    return r
