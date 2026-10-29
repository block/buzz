import 'package:flutter/foundation.dart';

import '../../../shared/deeplink/deep_link.dart';
import '../../../shared/mentions/mention_bindings.dart';
import '../message_media.dart';
import 'link_normalizer.dart';

/// One piece of a message preview: plain text or an inline token that renders
/// as a chip or image.
@immutable
sealed class PreviewPiece {
  const PreviewPiece();
}

final class PreviewText extends PreviewPiece {
  final String text;
  const PreviewText(this.text);

  @override
  bool operator ==(Object other) => other is PreviewText && other.text == text;
  @override
  int get hashCode => text.hashCode;
  @override
  String toString() => 'PreviewText($text)';
}

final class PreviewMention extends PreviewPiece {
  final String pubkey;
  final String label;

  /// The full label, before shortening a pubkey qualifier.
  final String semanticsLabel;
  const PreviewMention({
    required this.pubkey,
    required this.label,
    required this.semanticsLabel,
  });

  @override
  bool operator ==(Object other) =>
      other is PreviewMention &&
      other.pubkey == pubkey &&
      other.label == label &&
      other.semanticsLabel == semanticsLabel;
  @override
  int get hashCode => Object.hash(pubkey, label, semanticsLabel);
  @override
  String toString() => 'PreviewMention($pubkey, $label)';
}

final class PreviewEmoji extends PreviewPiece {
  final String shortcode;
  final String url;
  const PreviewEmoji({required this.shortcode, required this.url});

  @override
  bool operator ==(Object other) =>
      other is PreviewEmoji && other.shortcode == shortcode && other.url == url;
  @override
  int get hashCode => Object.hash(shortcode, url);
  @override
  String toString() => 'PreviewEmoji($shortcode)';
}

final class PreviewChannel extends PreviewPiece {
  final String name;
  const PreviewChannel(this.name);

  @override
  bool operator ==(Object other) =>
      other is PreviewChannel && other.name == name;
  @override
  int get hashCode => name.hashCode;
  @override
  String toString() => 'PreviewChannel($name)';
}

final class PreviewMedia extends PreviewPiece {
  final MessageMediaKind kind;
  const PreviewMedia(this.kind);

  @override
  bool operator ==(Object other) => other is PreviewMedia && other.kind == kind;
  @override
  int get hashCode => kind.hashCode;
  @override
  String toString() => 'PreviewMedia(${kind.name})';
}

/// The pieces of a [maxLines] preview of Markdown [content], without parsing
/// it as Markdown.
///
/// Block syntax is dropped line by line and inline emphasis is unwrapped, the
/// way desktop's `getMarkdownPreviewText` does, so the cost is linear in the
/// part of the message that can show. Each kept source line starts a new
/// line, so lines past the first `maxLines + 1` are never read; the extra one
/// keeps the overflow that ends the preview with an ellipsis. Inline code is
/// kept as literal text.
///
/// [mentionBindings] and [channelNames] are keyed by lowercase name,
/// [customEmoji] maps a lowercase shortcode to its image URL, and
/// [imetaByUrl] is keyed by normalized URL.
List<PreviewPiece> messagePreviewPieces(
  String content, {
  required int maxLines,
  Map<String, Set<String>> mentionBindings = const {},
  Map<String, String> mentionNames = const {},
  Map<String, String> mentionLabels = const {},
  Map<String, String> customEmoji = const {},
  Map<String, String> channelNames = const {},
  Map<String, ImetaEntry> imetaByUrl = const {},
}) {
  final builder = _PreviewBuilder(
    mentionBindings: mentionBindings,
    mentionNames: mentionNames,
    mentionLabels: mentionLabels,
    customEmoji: customEmoji,
    channelNames: channelNames,
    imetaByUrl: imetaByUrl,
  );
  var kept = 0;
  var fenceOpen = false;
  for (final source in content.split('\n')) {
    if (kept > maxLines) break;
    var line = source.trim();
    // A fence opens at any line starting with backticks and closes only at a
    // line holding nothing else, as in gpt_markdown.
    if (!fenceOpen && line.startsWith('```') || fenceOpen && line == '```') {
      fenceOpen = !fenceOpen;
      continue;
    }
    if (line.isEmpty) continue;
    if (!fenceOpen) {
      if (_rule.hasMatch(line) || _tableDivider.hasMatch(line)) continue;
      line = _blockPrefix(line);
      if (line.isEmpty) continue;
    }
    // A long line wraps past the visible lines well before this length.
    if (line.length > _maxLineLength) {
      line = line.substring(0, _maxLineLength);
    }
    if (kept > 0) builder.text('\n');
    kept++;
    if (fenceOpen) {
      builder.text(line);
    } else {
      builder.line(line);
    }
  }
  return builder.pieces;
}

const _maxLineLength = 2000;

final _rule = RegExp(r'^([-*_])(?:\s*\1){2,}$');
final _tableDivider = RegExp(r'^\|?(?:\s*:?-+:?\s*\|)+\s*:?-*:?\s*\|?$');
final _quote = RegExp(r'^(?:>\s?)+');
final _heading = RegExp(r'^#{1,6}\s+');
final _task = RegExp(r'^[-*+]\s+\[[ xX]\]\s+');
final _bullet = RegExp(r'^[-*+]\s+');
final _ordered = RegExp(r'^\d+[.)]\s+');

String _blockPrefix(String line) {
  var result = line
      .replaceFirst(_quote, '')
      .replaceFirst(_heading, '')
      .replaceFirst(_task, '')
      .replaceFirst(_bullet, '')
      .replaceFirst(_ordered, '');
  if (result.startsWith('|')) {
    result = result
        .replaceFirst(RegExp(r'^\|'), '')
        .replaceFirst(RegExp(r'\|$'), '')
        .split('|')
        .map((cell) => cell.trim())
        .where((cell) => cell.isNotEmpty)
        .join('  ');
  }
  return result.trim();
}

final _link = RegExp(r'(!?)\[([^\]]*)\]\(<?([^)\s>]+)>?(?:\s+"[^"]*")?\)');

// Emphasis is unwrapped only when its delimiters hug the text, and `_` only
// at word boundaries, so snake_case names and emoji shortcodes survive.
final _emphasis = [
  RegExp(r'\*\*(?=\S)(.+?)(?<=\S)\*\*'),
  RegExp(r'(?<![A-Za-z0-9_])__(?=\S)(.+?)(?<=\S)__(?![A-Za-z0-9_])'),
  RegExp(r'\*(?=\S)(.+?)(?<=\S)\*'),
  RegExp(r'(?<![A-Za-z0-9_])_(?=\S)(.+?)(?<=\S)_(?![A-Za-z0-9_])'),
  RegExp(r'~~(?=\S)(.+?)(?<=\S)~~'),
];

String _unwrapEmphasis(String text) {
  var result = text;
  for (final pattern in _emphasis) {
    result = result.replaceAllMapped(pattern, (m) => m[1]!);
  }
  return result;
}

// Look ahead so adjacent tokens sharing a colon are both considered.
final _shortcode = RegExp(r'(?=:([a-z0-9_-]+):)', caseSensitive: false);
final _qualifier = RegExp(r'\(([0-9a-f]{64})\)');

class _PreviewBuilder {
  final Map<String, Set<String>> mentionBindings;
  final Map<String, String> mentionNames;
  final Map<String, String> mentionLabels;
  final Map<String, String> customEmoji;
  final Map<String, String> channelNames;
  final Map<String, ImetaEntry> imetaByUrl;
  late final RegExp _channel = _channelPattern(channelNames.keys);
  final pieces = <PreviewPiece>[];

  _PreviewBuilder({
    required this.mentionBindings,
    required this.mentionNames,
    required this.mentionLabels,
    required this.customEmoji,
    required this.channelNames,
    required this.imetaByUrl,
  });

  void text(String text) {
    if (text.isEmpty) return;
    final last = pieces.isEmpty ? null : pieces.last;
    if (last is PreviewText) {
      pieces[pieces.length - 1] = PreviewText(last.text + text);
    } else {
      pieces.add(PreviewText(text));
    }
  }

  void add(PreviewPiece piece) =>
      piece is PreviewText ? text(piece.text) : pieces.add(piece);

  /// One line of prose: inline code is literal, everything else is inline
  /// Markdown.
  void line(String line) {
    final parts = line.split('`');
    // An unpaired backtick is literal.
    if (parts.length.isEven) {
      parts[parts.length - 2] += '`${parts.removeLast()}';
    }
    for (var i = 0; i < parts.length; i++) {
      if (i.isOdd) {
        text(parts[i]);
      } else {
        _inline(parts[i]);
      }
    }
  }

  void _inline(String source) {
    var start = 0;
    for (final match in _link.allMatches(source)) {
      _tokens(_unwrapEmphasis(source.substring(start, match.start)));
      start = match.end;
      final isImage = match[1]!.isNotEmpty;
      final label = match[2]!;
      final url = match[3]!;
      final imeta = imetaByUrl[normalizeMarkdownDestination(url)];
      final kind =
          classifyMediaUrl(url, imeta: imeta) ??
          (isImage ? MessageMediaKind.image : null);
      if (kind != null) {
        add(PreviewMedia(kind));
        continue;
      }
      final uri = Uri.tryParse(url);
      final deepLink = uri?.scheme == 'buzz' ? parseBuzzDeepLink(uri!) : null;
      if (deepLink is ChannelDeepLink) {
        add(PreviewChannel(_channelName(deepLink.channelId)));
        continue;
      }
      _tokens(_unwrapEmphasis(label.isEmpty ? url : label));
    }
    _tokens(_unwrapEmphasis(source.substring(start)));
  }

  String _channelName(String channelId) {
    for (final entry in channelNames.entries) {
      if (entry.value == channelId) return entry.key;
    }
    return channelId.substring(0, channelId.length < 8 ? channelId.length : 8);
  }

  /// Plain text with mention, custom emoji and channel tokens.
  void _tokens(String source) {
    if (source.isEmpty) return;
    final found = <({int start, int end, PreviewPiece piece})>[];
    if (mentionBindings.isNotEmpty && source.contains('@')) {
      for (final range in mentionOccurrences(source, mentionBindings.keys)) {
        final piece = _mention(
          range.label,
          source.substring(range.start + 1, range.end),
        );
        if (piece != null) {
          found.add((start: range.start, end: range.end, piece: piece));
        }
      }
    }
    if (customEmoji.isNotEmpty && source.contains(':')) {
      for (final match in _shortcode.allMatches(source)) {
        final shortcode = match[1]!.toLowerCase();
        final url = customEmoji[shortcode];
        if (url == null) continue;
        found.add((
          start: match.start,
          end: match.start + shortcode.length + 2,
          piece: PreviewEmoji(shortcode: shortcode, url: url),
        ));
      }
    }
    if (source.contains('#')) {
      for (final match in _channel.allMatches(source)) {
        found.add((
          start: match.start,
          end: match.end,
          piece: PreviewChannel(match[0]!.substring(1)),
        ));
      }
    }
    found.sort((a, b) => a.start.compareTo(b.start));
    var cursor = 0;
    for (final token in found) {
      if (token.start < cursor) continue;
      text(source.substring(cursor, token.start));
      add(token.piece);
      cursor = token.end;
    }
    text(source.substring(cursor));
  }

  /// Mirrors the thread view's mention chip: an ambiguous label stays text.
  PreviewMention? _mention(String label, String typed) {
    final matches = mentionBindings[label] ?? const <String>{};
    if (matches.length != 1) return null;
    final pubkey = matches.single;
    final displayName = _qualifier.hasMatch(label)
        ? typed
        : mentionLabels[pubkey] ?? mentionNames[pubkey];
    final fullLabel = displayName ?? typed;
    return PreviewMention(
      pubkey: pubkey,
      label: fullLabel.replaceAllMapped(
        _qualifier,
        (m) => '(${m[1]!.substring(0, 8)}…${m[1]!.substring(60)})',
      ),
      semanticsLabel: fullLabel,
    );
  }
}

/// The thread view's channel token: a known name, which may contain
/// characters the generic token excludes, or any `#token`.
RegExp _channelPattern(Iterable<String> knownNames) {
  final names =
      knownNames
          .map((name) => name.trim())
          .where((name) => name.isNotEmpty)
          .toSet()
          .toList()
        ..sort((a, b) => b.length.compareTo(a.length));
  const generic = r'[A-Za-z0-9_][A-Za-z0-9_-]*';
  const leading = r'(?<![\w./:-])';
  const trailing = r'(?=$|[\s,;.!?:)\]}])';
  final alternatives = [...names.map(RegExp.escape), generic].join('|');
  return RegExp('$leading#(?:$alternatives)$trailing', caseSensitive: false);
}
