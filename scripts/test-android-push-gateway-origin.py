#!/usr/bin/env python3
"""Run Android's production gateway Gradle hook without compiling an APK.

Requires Java and the repository's Gradle wrapper (or --gradle PATH). The fixture
registers empty Flutter compile tasks, then attaches the unchanged production
hook from app/build.gradle.kts. No Android SDK or signing credentials are used.
"""

import argparse
import base64
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    repo = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gradle", default=str(repo / "mobile/android/gradlew"))
    args = parser.parse_args()
    source = (repo / "mobile/android/app/build.gradle.kts").read_text()
    start = source.index('val dartDefines = providers.gradleProperty("dart-defines")')
    end = source.index("// Worktree-aware debug identity", start)
    hook = source[start:end]
    cases = [("Release", None, True), ("Profile", None, True)]
    for mode in ("Release", "Profile"):
        for origin in (
            "https://push.example",
            "https://push.example/",
            "https://push.example:1",
            "https://push.example:443",
            "https://push.example:8444",
            "https://push.example:65535",
        ):
            cases.append((mode, origin, True))
    cases.append(("Debug", "http://localhost:8080", True))
    for origin in (
        "",
        "not-a-url",
        "http://localhost:8080",
        "https://push.example:0",
        "https://push.example:65536",
        "https://push.example:invalid",
        "https://user:password@push.example:8444",
        "https://push.example:8444/path",
        "https://push.example:8444?token=x",
        "https://push.example:8444#fragment",
    ):
        cases.append(("Release", origin, False))
    cases.append(("Profile", "http://localhost:8080", False))
    with tempfile.TemporaryDirectory(prefix="buzz-android-gateway-") as tmp:
        fixture = Path(tmp)
        (fixture / "settings.gradle.kts").write_text(
            'rootProject.name = "gateway-origin-check"\n'
        )
        (fixture / "build.gradle.kts").write_text(
            "import java.net.URI\nimport java.util.Base64\n"
            'for (mode in listOf("Release", "Profile", "Debug")) {\n'
            '    tasks.register("compileFlutterBuild$mode") {\n'
            '        doLast { println("GATE_ACCEPTED") }\n    }\n}\n' + hook
        )
        for mode, origin, accepted in cases:
            properties = ""
            if origin is not None:
                encoded = base64.b64encode(
                    f"BUZZ_PUSH_GATEWAY_URL={origin}".encode()
                ).decode()
                properties = f"dart-defines={encoded}\n"
            (fixture / "gradle.properties").write_text(properties)
            # Isolate the fixture from any caller-supplied Gradle properties.
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith("ORG_GRADLE_PROJECT_")}
            result = subprocess.run(
                [args.gradle, "--console=plain", "--quiet", "--max-workers=2",
                 "--project-dir", str(fixture), f"compileFlutterBuild{mode}"],
                env=env, capture_output=True, text=True, timeout=180,
            )
            output = result.stdout + result.stderr
            if accepted:
                passed = result.returncode == 0 and "GATE_ACCEPTED" in output
            else:
                passed = (result.returncode != 0 and "GATE_ACCEPTED" not in output
                          and "BUZZ_PUSH_GATEWAY_URL must be an HTTPS origin"
                          in output)
            if not passed:
                raise SystemExit(
                    f"FAIL {mode} {origin!r}, expected accepted={accepted}\n{output}"
                )
            print(f"PASS {mode} {origin!r}: accepted={accepted}", flush=True)
    print(f"Passed {len(cases)} Android Gradle gateway cases.")


if __name__ == "__main__":
    main()
