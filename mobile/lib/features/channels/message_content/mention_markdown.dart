part of '../message_content.dart';

class _MentionMd {
  final Map<String, Set<String>> bindings;
  final Map<String, String> displayLabels;
  final Map<String, String> mentionNames;
  final Map<String, String> mentionLabels;
  final Set<String> agentMentionPubkeys;
  final void Function(String pubkey)? onMentionTap;
  late final RegExp _exp = _buildPrefixPattern(
    prefix: '@',
    knownNames: bindings.keys.map(_markdownMentionName),
    genericTokenPattern: r'[A-Za-z0-9_][A-Za-z0-9_\u00A0-]*',
  );

  _MentionMd({
    required this.bindings,
    required this.displayLabels,
    required this.mentionNames,
    required this.mentionLabels,
    required this.agentMentionPubkeys,
    this.onMentionTap,
  });

  /// Excluded from link labels: this component renders a [WidgetSpan], and a
  /// placeholder nested inside the link's own placeholder does not paint on
  /// iOS — an authored `[@mention](url)` renders as nothing. Link resolution
  /// wins over token detection inside a label.
  late final InlinePattern pattern = InlinePattern(
    pattern: _exp,
    builder: (context, match, style) => span(context, match[0]!, style),
  );

  RegExp get exp => _exp;

  InlineSpan span(BuildContext context, String text, TextStyle style) {
    final raw = exp.firstMatch(text.trim())?.group(0);
    if (raw == null) {
      return TextSpan(text: text, style: style);
    }

    final name = raw.substring(1).replaceAll('\u00A0', ' ').toLowerCase();
    final matches = bindings[name] ?? const <String>{};
    final pubkey = matches.length == 1 ? matches.single : null;
    if (bindings.containsKey(name) && matches.length != 1) {
      return TextSpan(text: text, style: style);
    }
    final displayName = name.contains(RegExp(r'\([0-9a-f]{64}\)'))
        ? displayLabels[name]
        : mentionLabels[pubkey] ?? mentionNames[pubkey];

    final isAgent =
        pubkey != null && agentMentionPubkeys.contains(pubkey.toLowerCase());
    final fullLabel = displayName ?? raw.substring(1);
    final visibleLabel = fullLabel.replaceAllMapped(
      RegExp(r'\(([0-9a-f]{64})\)'),
      (m) => '(${m[1]!.substring(0, 8)}…${m[1]!.substring(60)})',
    );
    final pill = MessageMentionPill(
      label: visibleLabel,
      semanticsLabel: fullLabel,
      isAgent: isAgent,
      textStyle: style,
    );

    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: pubkey != null && onMentionTap != null
          ? GestureDetector(onTap: () => onMentionTap!(pubkey), child: pill)
          : pill,
    );
  }
}

String _markdownMentionName(String name) => name.replaceAll(' ', '\u00A0');
