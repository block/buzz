#!/usr/bin/env python3
"""Keep the community-ban route inventory bound to production Axum routes."""

from __future__ import annotations

import ast
import csv
import re
import sys
from collections import Counter
from functools import lru_cache
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELAY_SRC = ROOT / "crates/buzz-relay/src"
ROUTE_INVENTORY = (
    ROOT / "crates/buzz-test-client/tests/fixtures/community-ban-route-inventory.tsv"
)
REGRESSION_INVENTORY = (
    ROOT / "crates/buzz-test-client/tests/fixtures/community-ban-regressions.tsv"
)
ROUTE_TEST = ROOT / "crates/buzz-test-client/tests/community_ban_routes.rs"
HTTP_METHODS = {
    "get": "GET",
    "head": "HEAD",
    "post": "POST",
    "put": "PUT",
    "delete": "DELETE",
    "patch": "PATCH",
    "options": "OPTIONS",
    "trace": "TRACE",
    "connect": "CONNECT",
    "any": "ANY",
}
AUTH_PLANES = {
    "dual_nip11_nip42",
    "public_metadata",
    "health_probe",
    "community_nip98",
    "community_blossom",
    "community_git_nip98",
    "community_nip42",
    "operator_control",
    "pre_membership_anonymous",
    "pre_membership_nip98",
    "internal_nip_fi",
    "workflow_webhook_secret",
    "testbed_internal",
    "internal_git_hook",
}
MEMBERSHIP_CLASSES = {
    "community",
    "conditional",
    "exempt",
    "internal",
    "not_applicable",
}
RESTRICTION_CLASSES = {"enforce", "conditional_enforce"}
REQUIRED_REGRESSIONS = {
    "http_membership",
    "root_admission",
    "root_admission_race",
    "root_ban_eviction",
    "root_owner_eviction",
    "root_fail_closed",
    "root_db_error",
    "audio_admission",
    "audio_admin_admission",
    "audio_owner_admission",
    "audio_local_eviction",
    "audio_admission_race",
    "tenant_local_eviction",
    "media_fail_closed",
    "git_fail_closed",
    "ingest_owner_ban",
    "moderation_owner_ban",
    "timeout_admin_denial",
    "http_tenant_isolation",
}
RAW_STRING_PREFIX = re.compile(r"(?:b)?r(#+)?\"")
CHAR_LITERAL = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|.)|[^'\\])'")


def mask_comments_and_strings(source: str) -> str:
    """Replace comments and string contents with spaces, preserving offsets."""
    out = list(source)
    i = 0
    while i < len(source):
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = len(source) if end < 0 else end
            for j in range(i, end):
                out[j] = " "
            i = end
            continue
        if source.startswith("/*", i):
            depth = 1
            j = i + 2
            while j < len(source) and depth:
                if source.startswith("/*", j):
                    depth += 1
                    j += 2
                elif source.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue

        raw_prefix = RAW_STRING_PREFIX.match(source, i)
        if raw_prefix:
            hashes = raw_prefix.group(1) or ""
            end_token = '"' + hashes
            start = i
            body_start = raw_prefix.end()
            end = source.find(end_token, body_start)
            end = len(source) if end < 0 else end + len(end_token)
            for j in range(start, end):
                if out[j] != "\n":
                    out[j] = " "
            i = end
            continue

        if source[i] == '"' or (
            source[i] == "b" and i + 1 < len(source) and source[i + 1] == '"'
        ):
            start = i
            quote = i if source[i] == '"' else i + 1
            j = quote + 1
            while j < len(source):
                if source[j] == "\\":
                    j += 2
                    continue
                if source[j] == '"':
                    j += 1
                    break
                j += 1
            for k in range(start, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue

        char_literal = CHAR_LITERAL.match(source, i)
        if char_literal:
            end = char_literal.end()
            for j in range(i, end):
                if out[j] != "\n":
                    out[j] = " "
            i = end
            continue

        i += 1
    return "".join(out)


def matching_delimiter(mask: str, start: int) -> int:
    opening = mask[start]
    pairs = {"(": ")", "[": "]", "{": "}"}
    closing = pairs[opening]
    depth = 0
    for index in range(start, len(mask)):
        if mask[index] == opening:
            depth += 1
        elif mask[index] == closing:
            depth -= 1
            if depth == 0:
                return index
    raise ValueError(f"unclosed {opening!r} at byte {start}")


def test_module_ranges(mask: str) -> list[tuple[int, int]]:
    ranges = []
    pattern = re.compile(
        r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*"
        r"(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?"
        r"mod\s+[A-Za-z_]\w*\s*\{"
    )
    for match in pattern.finditer(mask):
        brace = mask.find("{", match.start(), match.end())
        ranges.append((match.start(), matching_delimiter(mask, brace) + 1))
    return ranges


def in_ranges(position: int, ranges: list[tuple[int, int]]) -> bool:
    return any(start <= position < end for start, end in ranges)


def router_functions(path: Path, source: str, mask: str):
    pattern = re.compile(r"\bfn\s+([A-Za-z_]\w*)\s*\(")
    excluded = test_module_ranges(mask)
    relative = path.relative_to(RELAY_SRC).with_suffix("")
    module_parts = list(relative.parts)
    if module_parts[-1] == "mod":
        module_parts.pop()
    module = "::".join(module_parts)

    for match in pattern.finditer(mask):
        name = match.group(1)
        if name != "router" and not name.endswith("_router"):
            continue
        if in_ranges(match.start(), excluded):
            continue
        paren = mask.find("(", match.start(), match.end())
        close_paren = matching_delimiter(mask, paren)
        brace = mask.find("{", close_paren + 1)
        if brace < 0:
            continue
        end = matching_delimiter(mask, brace) + 1
        yield f"{module}::{name}", name, match.start(), brace, end


def split_first_top_level_comma(mask: str, start: int, end: int) -> int | None:
    stack: list[str] = []
    pairs = {"(": ")", "[": "]", "{": "}"}
    for index in range(start, end):
        char = mask[index]
        if char in pairs:
            stack.append(pairs[char])
        elif stack and char == stack[-1]:
            stack.pop()
        elif char == "," and not stack:
            return index
    return None


def string_value(expression: str) -> str | None:
    expression = expression.strip()
    if expression.startswith('"'):
        try:
            value = ast.literal_eval(expression)
        except (SyntaxError, ValueError):
            return None
        return value if isinstance(value, str) else None
    raw = re.fullmatch(r'r(#+)?"(.*)"\1', expression, re.DOTALL)
    return raw.group(2) if raw else None


def route_constants() -> dict[str, str]:
    constants = {}
    pattern = re.compile(
        r"(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Za-z_]\w*)\s*:\s*&str\s*=\s*"
        r'("(?:\\.|[^"\\])*"|r(?:#+)?".*?"(?:#+)?)\s*;',
        re.DOTALL,
    )
    for path in RELAY_SRC.rglob("*.rs"):
        if "tests" in path.parts or path.name.endswith("_tests.rs"):
            continue
        for match in pattern.finditer(path.read_text()):
            value = string_value(match.group(2))
            if value is not None:
                constants[match.group(1)] = value
    return constants


def route_path(expression: str, constants: dict[str, str]) -> str:
    literal = string_value(expression)
    if literal is not None:
        return literal
    constant = re.search(r"([A-Za-z_]\w*)\s*$", expression.strip())
    if constant and constant.group(1) in constants:
        return constants[constant.group(1)]
    raise ValueError(f"route path is not a literal or known string constant: {expression.strip()}")


def registered_calls(mask: str, source: str, function_start: int, function_end: int, call: str):
    pattern = re.compile(rf"\.\s*{re.escape(call)}\s*\(")
    for match in pattern.finditer(mask, function_start, function_end):
        opening = mask.find("(", match.start(), match.end())
        closing = matching_delimiter(mask, opening)
        comma = split_first_top_level_comma(mask, opening + 1, closing)
        if comma is None:
            continue
        yield (
            source[opening + 1 : comma].strip(),
            mask[comma + 1 : closing],
            source[comma + 1 : closing],
        )


def builder_prefixes(builders) -> dict[str, str]:
    main = next(
        (builder for builder in builders if builder[0] == "router::build_router"),
        None,
    )
    if main is None:
        raise ValueError("could not find relay router::build_router")
    _, _, path, source, mask, start, _, end = main
    prefixes = {}
    for first, second_mask, second_source in registered_calls(mask, source, start, end, "nest"):
        prefix = route_path(first, route_constants())
        target_match = re.search(r"((?:[A-Za-z_]\w*::)*[A-Za-z_]\w*)\s*\(", second_mask)
        if prefix is None or target_match is None:
            continue
        target = target_match.group(1)
        candidates = [b[0] for b in builders if b[0] == target or b[0].endswith(f"::{target}")]
        if len(candidates) == 1:
            prefixes[candidates[0]] = prefix
    return prefixes


def collect_registered_routes() -> Counter:
    constants = route_constants()
    builders = []
    for path in RELAY_SRC.rglob("*.rs"):
        if "tests" in path.parts or path.name.endswith("_tests.rs"):
            continue
        source = path.read_text()
        mask = mask_comments_and_strings(source)
        for qualified_name, name, start, brace, end in router_functions(path, source, mask):
            builders.append((qualified_name, name, path, source, mask, brace, start, end))

    prefixes = builder_prefixes(builders)
    registrations: Counter = Counter()
    errors = []
    for path in RELAY_SRC.rglob("*.rs"):
        if "tests" in path.parts or path.name.endswith("_tests.rs"):
            continue
        source = path.read_text()
        mask = mask_comments_and_strings(source)
        test_ranges = test_module_ranges(mask)
        router_ranges = [
            (builder[6], builder[7]) for builder in builders if builder[2] == path
        ]
        for match in re.finditer(r"\.\s*route\s*\(", mask):
            if in_ranges(match.start(), test_ranges):
                continue
            if not any(start <= match.start() < end for start, end in router_ranges):
                line = source.count("\n", 0, match.start()) + 1
                errors.append(
                    f"{path.relative_to(ROOT)}:{line}: production .route registration "
                    "is outside a recognized router builder"
                )

    for qualified_name, name, path, source, mask, _, start, end in builders:
        listener = "health" if qualified_name == "router::build_health_router" else "relay"
        prefix = prefixes.get(qualified_name, "")
        for path_expression, method_mask, _ in registered_calls(mask, source, start, end, "route"):
            try:
                route = route_path(path_expression, constants)
            except ValueError as error:
                errors.append(f"{path.relative_to(ROOT)}::{name}: {error}")
                continue
            methods = {
                method.upper()
                for method in re.findall(
                    r"\b(get|head|post|put|delete|patch|options|trace|connect|any)\s*\(",
                    method_mask,
                )
            }
            if not methods:
                errors.append(
                    f"{path.relative_to(ROOT)}::{name}: no Axum HTTP method in route expression"
                )
                continue
            full_path = f"{prefix.rstrip('/')}{route}" if prefix else route
            for method in methods:
                registrations[(listener, method, full_path)] += 1

    if errors:
        raise ValueError("route scanner could not classify registrations:\n  " + "\n  ".join(errors))
    duplicates = [route for route, count in registrations.items() if count > 1]
    if duplicates:
        rendered = ", ".join(f"{scope} {method} {path}" for scope, method, path in duplicates)
        raise ValueError(f"duplicate production route registrations: {rendered}")
    return registrations


def read_tsv(path: Path, expected_header: list[str]) -> list[dict[str, str]]:
    with path.open(newline="") as file:
        reader = csv.DictReader(file, delimiter="\t")
        if reader.fieldnames != expected_header:
            raise ValueError(f"{path.relative_to(ROOT)} header must be {expected_header}")
        rows = list(reader)
    if any(None in row or any(value is None for value in row.values()) for row in rows):
        raise ValueError(f"{path.relative_to(ROOT)} contains a row with the wrong number of columns")
    return rows


@lru_cache(maxsize=None)
def source_mask(path: Path) -> str | None:
    if not path.is_file():
        return None
    return mask_comments_and_strings(path.read_text())


def test_function_exists(path: Path, name: str) -> bool:
    mask = source_mask(path)
    if mask is None:
        return False
    return re.search(rf"\bfn\s+{re.escape(name)}\s*\(", mask) is not None


def check_route_inventory() -> tuple[int, int, int]:
    routes = read_tsv(
        ROUTE_INVENTORY,
        [
            "id",
            "listener",
            "method",
            "path",
            "auth_plane",
            "membership",
            "restriction",
            "exercise",
            "control",
        ],
    )
    by_key: dict[tuple[str, str, str], dict[str, str]] = {}
    ids = set()
    matrix_cases = set()
    pending = []
    for row in routes:
        route_id = row["id"]
        if not route_id or route_id in ids:
            raise ValueError(f"duplicate or empty route id: {route_id!r}")
        ids.add(route_id)
        key = (row["listener"], row["method"], row["path"])
        if key in by_key:
            raise ValueError(f"duplicate inventory route: {key}")
        by_key[key] = row
        if row["listener"] not in {"relay", "health"}:
            raise ValueError(f"{route_id}: unknown listener {row['listener']!r}")
        if row["auth_plane"] not in AUTH_PLANES:
            raise ValueError(f"{route_id}: unknown auth plane {row['auth_plane']!r}")
        if row["membership"] not in MEMBERSHIP_CLASSES:
            raise ValueError(f"{route_id}: unknown membership class {row['membership']!r}")
        if row["method"] not in set(HTTP_METHODS.values()):
            raise ValueError(f"{route_id}: unknown HTTP method {row['method']!r}")

        restriction = row["restriction"]
        exercise = row["exercise"]
        if restriction in RESTRICTION_CLASSES:
            if not exercise or exercise == "classified":
                raise ValueError(f"{route_id}: enforced restriction needs a behavior exercise")
        elif restriction.startswith("not_applicable:"):
            if not restriction.partition(":")[2] or exercise != "classified":
                raise ValueError(f"{route_id}: restriction exemption needs a reason and classification")
        else:
            raise ValueError(f"{route_id}: unrecognized restriction class {restriction!r}")

        if row["membership"] == "exempt" and restriction.startswith("not_applicable:"):
            if route_id != "invite_accept_policy":
                raise ValueError(
                    f"{route_id}: membership exemption cannot silently exempt restriction"
                )
        if exercise.startswith("matrix:"):
            if row["control"] not in {"member", "admin", "owner"}:
                raise ValueError(f"{route_id}: route-matrix row needs an allowed control principal")
            matrix_cases.update(exercise.partition(":")[2].split("+"))
        elif exercise.startswith("e2e:"):
            test_name = exercise.partition(":")[2]
            if not test_function_exists(ROUTE_TEST, test_name):
                raise ValueError(f"{route_id}: missing E2E test {test_name!r}")
        elif exercise.startswith("pg:"):
            tests = exercise.partition(":")[2].split("+")
            for test_name in tests:
                found = any(
                    test_function_exists(path, test_name)
                    for path in RELAY_SRC.rglob("*.rs")
                    if "tests" not in path.parts and not path.name.endswith("_tests.rs")
                )
                if not found:
                    raise ValueError(f"{route_id}: missing PostgreSQL regression {test_name!r}")
        elif exercise.startswith("pending:"):
            pending.append((route_id, exercise))
        elif exercise != "classified":
            raise ValueError(f"{route_id}: unrecognized exercise classification {exercise!r}")

    registrations = collect_registered_routes()
    registered = set(registrations)
    inventoried = set(by_key)
    missing = sorted(registered - inventoried)
    stale = sorted(inventoried - registered)
    if missing or stale:
        details = []
        if missing:
            details.append("registered routes without inventory rows: " + ", ".join(map(str, missing)))
        if stale:
            details.append("inventory rows without registered routes: " + ", ".join(map(str, stale)))
        raise ValueError("; ".join(details))

    if set(pending) != {("invite_claim", "pending:BUZZ-268"), ("workflow_webhook", "pending:BUZZ-272")}:
        raise ValueError(
            "pending coverage must explicitly track BUZZ-268 invitations and BUZZ-272 webhook owner bans"
        )
    claim = next((row for row in routes if row["id"] == "invite_claim"), None)
    if claim is None or claim["membership"] != "exempt" or claim["restriction"] != "enforce":
        raise ValueError("invite claim must remain membership-exempt and restriction-enforced")
    acceptance = next((row for row in routes if row["id"] == "invite_accept_policy"), None)
    if acceptance is None or acceptance["membership"] != "exempt":
        raise ValueError("anonymous policy acceptance needs its separate pre-membership classification")

    route_test_mask = mask_comments_and_strings(ROUTE_TEST.read_text())
    supported = re.search(
        r"const\s+COMMUNITY_BAN_MATRIX_CASES\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];",
        route_test_mask,
        re.DOTALL,
    )
    if supported is None:
        raise ValueError("route E2E test must declare COMMUNITY_BAN_MATRIX_CASES")
    supported_cases = set(re.findall(r'"([a-z_]+)"', ROUTE_TEST.read_text()[supported.start() : supported.end()]))
    if matrix_cases != supported_cases:
        raise ValueError(
            "route-matrix exercise cases differ from COMMUNITY_BAN_MATRIX_CASES: "
            f"inventory-only={sorted(matrix_cases - supported_cases)}, "
            f"test-only={sorted(supported_cases - matrix_cases)}"
        )
    if "community_http_ban_matrix" not in route_test_mask:
        raise ValueError("route inventory is not exercised by community_http_ban_matrix")
    return len(routes), len(matrix_cases), len(pending)


def check_regression_inventory() -> int:
    rows = read_tsv(
        REGRESSION_INVENTORY,
        ["id", "file", "test", "lane", "property"],
    )
    seen = set()
    for row in rows:
        if not row["id"] or row["id"] in seen:
            raise ValueError(f"duplicate or empty supporting regression id: {row['id']!r}")
        seen.add(row["id"])
        if row["lane"] not in {"unit", "postgres-ci", "relay-e2e"}:
            raise ValueError(f"{row['id']}: unknown validation lane {row['lane']!r}")
        path = ROOT / row["file"]
        if not test_function_exists(path, row["test"]):
            raise ValueError(f"{row['id']}: missing referenced test {row['file']}::{row['test']}")
    missing = REQUIRED_REGRESSIONS - seen
    if missing:
        raise ValueError(f"supporting regression inventory is missing {sorted(missing)}")
    return len(rows)


def main() -> int:
    try:
        route_count, case_count, pending_count = check_route_inventory()
        regression_count = check_regression_inventory()
    except (OSError, ValueError, csv.Error) as error:
        print(f"community-ban route inventory failed: {error}", file=sys.stderr)
        return 1
    print(
        "community-ban route inventory passed: "
        f"{route_count} registered routes classified, {case_count} HTTP behavior cases, "
        f"{regression_count} supporting regressions indexed, {pending_count} sibling implementation cases pending"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
