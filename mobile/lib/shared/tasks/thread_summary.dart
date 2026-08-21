/// Client-side thread summarization.
///
/// **This is an extractive digest, not a generative one.** The mobile app has
/// no LLM: `pubspec.yaml` declares no model SDK, nothing under `lib/` talks to
/// a completion endpoint, and the only agent output mobile consumes is the
/// read-only kind:24200 observer stream. So rather than pretend, [summarizeThread]
/// selects and reorganizes the thread's own most load-bearing lines into a
/// Markdown digest.
///
/// Everything here is pure and deterministic — same input, same output, no
/// clock, no network, no `Random`. That is what makes it unit-testable, and it
/// is also what makes the digest safe to persist as a task's
/// `summary_persisted` event: two clients summarizing the same thread agree.
library;

import 'package:flutter/foundation.dart';

/// One message handed to [summarizeThread].
@immutable
class ThreadMessageDigest {
  /// Pairs a display [author] with their message [text].
  const ThreadMessageDigest({required this.author, required this.text});

  /// Display name of whoever wrote the message.
  final String author;

  /// Raw message body, Markdown included.
  final String text;
}

/// Words that mark a line as carrying a decision, commitment, or blocker.
///
/// Matched as substrings against lowercased text so inflections are covered by
/// one stem (`decid` catches "decide", "decided", "deciding").
const _decisionMarkers = <String>[
  'decid',
  'agree',
  'let us',
  "let's",
  'we should',
  'we will',
  "we'll",
  'i will',
  "i'll",
  'plan is',
  'next step',
  'action item',
  'todo',
  'to do',
  'blocked',
  'blocker',
  'ship',
  'deadline',
  'due ',
  'owner',
  'assign',
  'merged',
  'fixed',
  'root cause',
  'conclusion',
];

final _urlPattern = RegExp(r'https?://[^\s<>()\[\]]+');
final _fencedCodePattern = RegExp(r'```[\s\S]*?```');
final _inlineCodePattern = RegExp('`[^`]*`');
final _whitespacePattern = RegExp(r'\s+');
final _trailingPunctuation = RegExp(r'[.,;:!)\]}>"’”]+$');

/// Builds a Markdown digest of [messages].
///
/// Returns a "nothing to summarize" line rather than an empty string for an
/// empty or all-blank thread, so a caller can always show the result.
///
/// [maxHighlights] caps the statement bullets and [maxOpenQuestions] the
/// question bullets; [maxCharsPerLine] is where a quoted line is elided.
String summarizeThread(
  List<ThreadMessageDigest> messages, {
  int maxHighlights = 5,
  int maxOpenQuestions = 3,
  int maxCharsPerLine = 180,
}) {
  final entries = <_ScoredLine>[];
  final authors = <String>[];
  final links = <String>[];

  for (var index = 0; index < messages.length; index++) {
    final message = messages[index];
    final author = _cleanAuthor(message.author);
    final text = _condense(message.text);
    if (text.isEmpty) continue;

    if (!authors.contains(author)) authors.add(author);
    for (final url in _extractLinks(message.text)) {
      if (!links.contains(url)) links.add(url);
    }
    entries.add(
      _ScoredLine(
        order: index,
        author: author,
        text: text,
        isQuestion: text.endsWith('?'),
        score: _score(text),
      ),
    );
  }

  if (entries.isEmpty) return 'No messages to summarize yet.';

  final questions = _pick(
    entries.where((entry) => entry.isQuestion),
    maxOpenQuestions,
  );
  final statements = _pick(
    entries.where((entry) => !entry.isQuestion),
    maxHighlights,
  );

  final lines = <String>[
    '## Thread summary',
    '',
    _headline(messageCount: entries.length, authors: authors),
  ];

  // A thread of only questions has no statements to highlight; a thread with
  // no questions has no open-questions section. Both are normal, so each
  // section is emitted only when it has content.
  if (statements.isNotEmpty) {
    lines
      ..add('')
      ..add('**Highlights**')
      ..add('');
    for (final entry in statements) {
      lines.add('- ${entry.bullet(maxCharsPerLine)}');
    }
  }

  if (questions.isNotEmpty) {
    lines
      ..add('')
      ..add('**Open questions**')
      ..add('');
    for (final entry in questions) {
      lines.add('- ${entry.bullet(maxCharsPerLine)}');
    }
  }

  if (links.isNotEmpty) {
    lines
      ..add('')
      ..add('**Links**')
      ..add('');
    for (final link in links) {
      lines.add('- $link');
    }
  }

  return lines.join('\n');
}

/// Selects the [limit] highest-scoring lines, then restores thread order.
///
/// Two-stage on purpose: relevance decides *which* lines survive, chronology
/// decides how they read. Ties break toward the earlier message, so the result
/// never depends on iteration order.
List<_ScoredLine> _pick(Iterable<_ScoredLine> candidates, int limit) {
  if (limit <= 0) return const [];
  final ranked = candidates.toList()
    ..sort((a, b) {
      final byScore = b.score.compareTo(a.score);
      return byScore != 0 ? byScore : a.order.compareTo(b.order);
    });
  final selected = ranked.take(limit).toList()
    ..sort((a, b) => a.order.compareTo(b.order));
  return selected;
}

String _headline({required int messageCount, required List<String> authors}) {
  final messageLabel = messageCount == 1
      ? '1 message'
      : '$messageCount messages';
  return '_$messageLabel from ${_formatAuthors(authors)}._';
}

/// Renders an author list as prose, collapsing long rosters.
String _formatAuthors(List<String> authors) {
  if (authors.isEmpty) return 'nobody';
  if (authors.length == 1) return authors.single;
  if (authors.length == 2) return '${authors[0]} and ${authors[1]}';
  if (authors.length <= 4) {
    final head = authors.sublist(0, authors.length - 1).join(', ');
    return '$head and ${authors.last}';
  }
  final remaining = authors.length - 3;
  final others = remaining == 1 ? '1 other' : '$remaining others';
  return '${authors.take(3).join(', ')} and $others';
}

/// Scores a line by how much it looks like the point of the thread.
int _score(String text) {
  final lowered = text.toLowerCase();
  var score = 0;
  for (final marker in _decisionMarkers) {
    if (lowered.contains(marker)) score += 2;
  }
  if (_urlPattern.hasMatch(text)) score += 2;
  // Longer lines carry more, but only up to a point — a wall of text is not
  // three times the signal of a sentence.
  final words = text.split(' ').where((word) => word.isNotEmpty).length;
  score += (words ~/ 8).clamp(0, 3);
  return score;
}

/// Collapses a message body to a single quotable line.
///
/// Code is replaced rather than quoted: a fenced block would break the bullet
/// list it is being inlined into, and its contents are rarely the summary.
String _condense(String raw) {
  return raw
      .replaceAll(_fencedCodePattern, ' [code] ')
      .replaceAll(_inlineCodePattern, ' [code] ')
      .replaceAll(_whitespacePattern, ' ')
      .trim();
}

String _cleanAuthor(String raw) {
  final trimmed = raw.replaceAll(_whitespacePattern, ' ').trim();
  return trimmed.isEmpty ? 'Unknown' : trimmed;
}

List<String> _extractLinks(String raw) {
  return [
    for (final match in _urlPattern.allMatches(raw))
      match.group(0)!.replaceFirst(_trailingPunctuation, ''),
  ];
}

/// Longest title [threadTaskTitle] will produce, comfortably inside the
/// relay's 200-character ceiling.
const _maxDerivedTitleChars = 120;

/// Derives a task title from the thread's opening message.
///
/// Used when a summary is saved to a brand-new task: the thread's first line is
/// what a human would have typed as the title anyway.
String threadTaskTitle(List<ThreadMessageDigest> messages) {
  for (final message in messages) {
    final text = _condense(message.text);
    if (text.isNotEmpty) return elideSummaryLine(text, _maxDerivedTitleChars);
  }
  return 'Thread summary';
}

/// Elides [text] at [maxChars] on a word boundary when possible.
String elideSummaryLine(String text, int maxChars) {
  if (maxChars <= 1 || text.length <= maxChars) return text;
  final clipped = text.substring(0, maxChars - 1);
  final lastSpace = clipped.lastIndexOf(' ');
  // Only honour a word boundary that is not pathologically early, otherwise a
  // single very long token would collapse the line to almost nothing.
  final cut = lastSpace > maxChars ~/ 2 ? lastSpace : clipped.length;
  return '${clipped.substring(0, cut).trimRight()}…';
}

@immutable
class _ScoredLine {
  const _ScoredLine({
    required this.order,
    required this.author,
    required this.text,
    required this.isQuestion,
    required this.score,
  });

  final int order;
  final String author;
  final String text;
  final bool isQuestion;
  final int score;

  String bullet(int maxChars) =>
      '**$author:** ${elideSummaryLine(text, maxChars)}';
}
