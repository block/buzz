"""Publish exact-SHA native-review evidence and timecoded highlights to Buzz."""
from __future__ import annotations

import hashlib
import json
import pathlib
import shutil
import subprocess
from typing import Any


class PublishError(RuntimeError):
    pass


def _artifact(run_dir: pathlib.Path, name: str) -> pathlib.Path:
    candidate = (run_dir / name).resolve()
    if run_dir.resolve() not in candidate.parents or not candidate.is_file():
        raise PublishError(f"artifact is missing or escapes receipt directory: {name}")
    return candidate


def _run(command: list[str]) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or f"exit {result.returncode}"
        raise PublishError(f"command failed: {detail}")
    return result


def _accepted_event(result: subprocess.CompletedProcess[str], operation: str) -> str:
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise PublishError(f"{operation} returned invalid JSON") from exc
    event_id = payload.get("event_id")
    if payload.get("accepted") is not True or not isinstance(event_id, str) or len(event_id) != 64:
        raise PublishError(f"{operation} was not accepted: {payload}")
    return event_id


def _sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _video_duration(video: pathlib.Path) -> float:
    ffprobe = shutil.which("ffprobe")
    if not ffprobe:
        raise PublishError("ffprobe is required to validate highlight timecodes")
    result = _run([
        ffprobe, "-v", "error", "-show_entries", "format=duration",
        "-of", "default=noprint_wrappers=1:nokey=1", str(video),
    ])
    try:
        duration = float(result.stdout.strip())
    except ValueError as exc:
        raise PublishError("ffprobe returned an invalid video duration") from exc
    if duration <= 0:
        raise PublishError("share video has no positive duration")
    return duration


def _load_highlights(path: pathlib.Path | None, duration: float) -> list[dict[str, Any]]:
    if path is None:
        return []
    try:
        payload = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise PublishError(f"cannot read highlights {path}: {exc}") from exc
    if not isinstance(payload, list):
        raise PublishError("highlights must be a JSON array")
    highlights: list[dict[str, Any]] = []
    for index, item in enumerate(payload):
        if not isinstance(item, dict) or set(item) != {"seconds", "text"}:
            raise PublishError(f"highlight {index} requires exactly seconds and text")
        seconds, text = item["seconds"], item["text"]
        if (not isinstance(seconds, (int, float)) or isinstance(seconds, bool)
                or seconds < 0 or seconds > duration):
            raise PublishError(f"highlight {index} seconds must be within the video (0..{duration:.3f})")
        if not isinstance(text, str) or not text.strip():
            raise PublishError(f"highlight {index} text must be non-empty")
        highlights.append({"seconds": float(seconds), "text": text.strip()})
    return highlights


def format_timecode(seconds: float) -> str:
    total_ms = round(seconds * 1000)
    hours, remainder = divmod(total_ms, 3_600_000)
    minutes, remainder = divmod(remainder, 60_000)
    whole_seconds, milliseconds = divmod(remainder, 1000)
    if hours:
        base = f"{hours}:{minutes:02d}:{whole_seconds:02d}"
    else:
        base = f"{minutes:02d}:{whole_seconds:02d}"
    return f"{base}.{milliseconds:03d}" if milliseconds else base


def publish_review(receipt_path: pathlib.Path, summary_path: pathlib.Path, channel: str,
                   reply_to: str, highlights_path: pathlib.Path | None = None,
                   mentions: list[str] | None = None) -> dict[str, Any]:
    buzz = shutil.which("buzz")
    if not buzz:
        raise PublishError("buzz CLI is required to publish review evidence")
    try:
        receipt = json.loads(receipt_path.read_text())
        summary = summary_path.read_text().strip()
    except (OSError, json.JSONDecodeError) as exc:
        raise PublishError(f"cannot read review input: {exc}") from exc
    if not summary:
        raise PublishError("review summary must be non-empty")
    provenance = receipt.get("provenance", {})
    if provenance.get("dirty") is not False:
        raise PublishError("review publication requires a clean source receipt")
    head_sha = provenance.get("head_sha")
    if not isinstance(head_sha, str) or len(head_sha) != 40:
        raise PublishError("receipt has no full head SHA")
    if receipt.get("cleanup", {}).get("status") != "passed":
        raise PublishError("review publication requires passed cleanup")
    share_name = receipt.get("artifacts", {}).get("share_video")
    if not isinstance(share_name, str):
        raise PublishError("receipt has no relay-safe share video")
    video = _artifact(receipt_path.parent, share_name)
    duration = _video_duration(video)
    highlights = _load_highlights(highlights_path, duration)
    evidence = (
        f"\n\n**Native evidence:** `{receipt.get('flow')}` · `{receipt.get('status')}` · "
        f"exact clean SHA `{head_sha}` · {duration:.3f}s."
    )
    command = [buzz, "messages", "send", "--channel", channel, "--reply-to", reply_to,
               "--content", summary + evidence, "--file", str(video)]
    for mention in mentions or []:
        command.extend(["--mention", mention])
    video_event_id = _accepted_event(_run(command), "review evidence publication")
    highlight_event_ids = []
    for highlight in highlights:
        content = f"[{format_timecode(highlight['seconds'])}] {highlight['text']}"
        event_id = _accepted_event(_run([
            buzz, "messages", "send", "--channel", channel, "--reply-to", video_event_id,
            "--content", content,
        ]), f"highlight at {highlight['seconds']:.3f}s")
        highlight_event_ids.append(event_id)
    return {
        "video_event_id": video_event_id,
        "highlight_event_ids": highlight_event_ids,
        "head_sha": head_sha,
        "video_sha256": _sha256(video),
        "duration_seconds": duration,
    }
