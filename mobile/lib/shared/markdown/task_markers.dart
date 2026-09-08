/// Pure helpers for GFM task-list markers (`- [ ]` / `- [x]`) inside message
/// Markdown.
///
/// The desktop client writes and rewrites the very same syntax (see
/// `desktop/src/shared/lib/toggleTaskMarker.mjs`); the two must agree
/// exactly, or a box checked on one platform lands on a different line on the
/// other. Two rules carry that agreement and are the reason this is not a
/// one-line regex:
///
/// * markers inside fenced code blocks are skipped — they render as code, not
///   as checkboxes, so counting them would shift every ordinal after the
///   fence;
/// * the closing bracket must be followed by whitespace or end-of-line, which
///   is the GFM rule (`- [ ]foo` is not a task).
library;

/// A list bullet (`-`, `*`, `+`, or `1.` / `1)`) followed by `[ ]` or `[x]`.
final RegExp _taskMarker = RegExp(
  r'^([ \t]*(?:[-*+]|\d{1,9}[.)])[ \t]+)\[[ xX]\](?=[ \t]|$)',
);

/// Opening or closing fence of a code block: three or more backticks/tildes.
final RegExp _fence = RegExp(r'^[ \t]*(`{3,}|~{3,})');

/// Flips the [taskIndex]-th task marker in [source] to [checked].
///
/// Returns `null` when the ordinal resolves to nothing, which means the source
/// no longer matches what was rendered — a concurrent edit landed between the
/// draw and the tap. Callers must not write a guess in that case.
String? toggleTaskMarker(String source, int taskIndex, bool checked) {
  if (taskIndex < 0) return null;

  final lines = source.split('\n');
  String? openFence;
  var seen = 0;

  for (var i = 0; i < lines.length; i++) {
    final line = lines[i];

    final fence = _fence.firstMatch(line);
    if (fence != null) {
      final char = fence.group(1)![0];
      if (openFence == null) {
        openFence = char;
      } else if (openFence == char) {
        openFence = null;
      }
      continue;
    }
    if (openFence != null) continue;

    final marker = _taskMarker.firstMatch(line);
    if (marker == null) continue;
    if (seen != taskIndex) {
      seen++;
      continue;
    }

    final bullet = marker.group(1)!;
    final rest = line.substring(marker.group(0)!.length);
    lines[i] = '$bullet[${checked ? 'x' : ' '}]$rest';
    return lines.join('\n');
  }

  return null;
}

/// Number of task markers the renderer will turn into checkboxes.
int countTaskMarkers(String source) {
  String? openFence;
  var count = 0;
  for (final line in source.split('\n')) {
    final fence = _fence.firstMatch(line);
    if (fence != null) {
      final char = fence.group(1)![0];
      if (openFence == null) {
        openFence = char;
      } else if (openFence == char) {
        openFence = null;
      }
      continue;
    }
    if (openFence != null) continue;
    if (_taskMarker.hasMatch(line)) count++;
  }
  return count;
}
