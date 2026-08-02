#!/usr/bin/env python3
"""Validate the semantic iOS target/configuration table via macOS plutil."""

import json
import subprocess
import sys
from pathlib import Path

PBXPROJ = Path("mobile/ios/Runner.xcodeproj/project.pbxproj")
# This asserts plutil-resolved pbxproj declarations, not object-key uniqueness or
# the final xcconfig-expanded bundle identity.
EXPECTED_ROWS = """\
NotificationService Debug Flutter/Debug.xcconfig JMTDPW9CG3 NotificationService/NotificationService.entitlements $(BUNDLE_IDENTIFIER).NotificationService
NotificationService Profile Flutter/Release.xcconfig EYF346PHUG NotificationService/NotificationService.entitlements $(BUNDLE_IDENTIFIER).NotificationService
NotificationService Release Flutter/Release.xcconfig EYF346PHUG NotificationService/NotificationService.entitlements $(BUNDLE_IDENTIFIER).NotificationService
Runner Debug Flutter/Debug.xcconfig JMTDPW9CG3 Runner/Runner.entitlements $(BUNDLE_IDENTIFIER)
Runner Profile Flutter/Release.xcconfig EYF346PHUG Runner/Runner.entitlements $(BUNDLE_IDENTIFIER)
Runner Release Flutter/Release.xcconfig EYF346PHUG Runner/Runner.entitlements $(BUNDLE_IDENTIFIER)
RunnerTests Debug Target Support Files/Pods-RunnerTests/Pods-RunnerTests.debug.xcconfig - - $(BUNDLE_IDENTIFIER).RunnerTests
RunnerTests Profile Target Support Files/Pods-RunnerTests/Pods-RunnerTests.profile.xcconfig - - $(BUNDLE_IDENTIFIER).RunnerTests
RunnerTests Release Target Support Files/Pods-RunnerTests/Pods-RunnerTests.release.xcconfig - - $(BUNDLE_IDENTIFIER).RunnerTests
""".splitlines()


def parse_project(path: Path) -> dict:
    result = subprocess.run(
        ["plutil", "-convert", "json", "-o", "-", str(path)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or "no error detail"
        raise RuntimeError(f"plutil failed to parse {path}: {detail}")
    if not result.stdout:
        raise RuntimeError(f"plutil returned no JSON for {path}")
    return json.loads(result.stdout)


def semantic_rows(project: dict) -> list[str]:
    objects = project["objects"]
    root = objects[project["rootObject"]]
    rows = []
    for target_id in root["targets"]:
        target = objects[target_id]
        if target.get("isa") != "PBXNativeTarget":
            continue
        configuration_list = objects[target["buildConfigurationList"]]
        for configuration_id in configuration_list["buildConfigurations"]:
            configuration = objects[configuration_id]
            settings = configuration.get("buildSettings", {})
            base_reference = configuration.get("baseConfigurationReference")
            base_path = objects.get(base_reference, {}).get("path", "-")
            rows.append(
                " ".join(
                    [
                        target.get("name", "-"),
                        configuration.get("name", "-"),
                        base_path,
                        settings.get("DEVELOPMENT_TEAM", "-"),
                        settings.get("CODE_SIGN_ENTITLEMENTS", "-"),
                        settings.get("PRODUCT_BUNDLE_IDENTIFIER", "-"),
                    ]
                )
            )
    return sorted(rows)


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else PBXPROJ
    try:
        actual = semantic_rows(parse_project(path))
    except (KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1

    if actual != EXPECTED_ROWS:
        print("FAIL: unexpected iOS target configuration semantics", file=sys.stderr)
        print("expected:", *EXPECTED_ROWS, sep="\n", file=sys.stderr)
        print("actual:", *actual, sep="\n", file=sys.stderr)
        return 1

    print("iOS target configuration semantics match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
