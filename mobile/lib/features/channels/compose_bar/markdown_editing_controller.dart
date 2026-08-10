part of '../compose_bar.dart';

enum _MarkdownStyle { codeBlock, inlineCode, bold, italic, strikethrough }

class _MarkdownRule {
  final RegExp pattern;
  final _MarkdownStyle style;
  final bool parsesNestedFormatting;

  _MarkdownRule(
    String pattern,
    this.style, {
    this.parsesNestedFormatting = true,
  }) : pattern = RegExp(pattern);
}

class _MarkdownEditingController extends TextEditingController {
  final Set<String> _agentMentionNames = <String>{};

  static final _rules = [
    _MarkdownRule(
      r'```(?:\r?\n)?([\s\S]*?)(?:\r?\n)?```',
      _MarkdownStyle.codeBlock,
      parsesNestedFormatting: false,
    ),
    _MarkdownRule(
      r'`([^`\n]*?)`',
      _MarkdownStyle.inlineCode,
      parsesNestedFormatting: false,
    ),
    _MarkdownRule(r'\*\*([^\n]*?)\*\*', _MarkdownStyle.bold),
    _MarkdownRule(r'~~([^\n]*?)~~', _MarkdownStyle.strikethrough),
    _MarkdownRule(r'_([^_\n]*?)_', _MarkdownStyle.italic),
  ];

  /// Updates the known agent labels which should render as agent mention
  /// chips. The editor still stores the literal `@Name` text, matching the
  /// markdown sent to the relay.
  void setAgentMentionNames(Iterable<String> names) {
    final next = {
      for (final name in names)
        if (name.trim().isNotEmpty) name.trim().toLowerCase(),
    };
    if (setEquals(_agentMentionNames, next)) return;
    _agentMentionNames
      ..clear()
      ..addAll(next);
    notifyListeners();
  }

  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    final baseStyle = style ?? const TextStyle();
    final composingRange =
        withComposing && value.composing.isValid && !value.composing.isCollapsed
        ? value.composing
        : TextRange.empty;
    return TextSpan(
      style: baseStyle,
      children: _buildMarkdownSpans(
        context,
        text,
        baseStyle,
        sourceOffset: 0,
        composingRange: composingRange,
      ),
    );
  }

  List<InlineSpan> _buildMarkdownSpans(
    BuildContext context,
    String source,
    TextStyle inheritedStyle, {
    required int sourceOffset,
    required TextRange composingRange,
  }) {
    final spans = <InlineSpan>[];
    var offset = 0;

    while (offset < source.length) {
      final tail = source.substring(offset);
      _MarkdownRule? nextRule;
      RegExpMatch? nextMatch;

      for (final rule in _rules) {
        final match = rule.pattern.firstMatch(tail);
        if (match == null) continue;
        if (nextMatch == null || match.start < nextMatch.start) {
          nextRule = rule;
          nextMatch = match;
        }
      }

      if (nextRule == null || nextMatch == null) {
        spans.addAll(
          _buildTextSpans(
            context,
            source.substring(offset),
            inheritedStyle,
            sourceOffset + offset,
            composingRange,
          ),
        );
        break;
      }

      if (nextMatch.start > 0) {
        spans.addAll(
          _buildTextSpans(
            context,
            tail.substring(0, nextMatch.start),
            inheritedStyle,
            sourceOffset + offset,
            composingRange,
          ),
        );
      }

      final fullMatch = nextMatch.group(0)!;
      final content = nextMatch.group(1)!;
      final (contentStart, contentEnd) = _contentBounds(
        fullMatch,
        nextRule.style,
      );
      final contentStyle = _styleFor(context, inheritedStyle, nextRule.style);
      final matchOffset = sourceOffset + offset + nextMatch.start;

      spans.add(
        TextSpan(
          text: fullMatch.substring(0, contentStart),
          style: _hiddenSyntaxStyle(inheritedStyle),
        ),
      );
      if (nextRule.parsesNestedFormatting) {
        spans.addAll(
          _buildMarkdownSpans(
            context,
            content,
            contentStyle,
            sourceOffset: matchOffset + contentStart,
            composingRange: composingRange,
          ),
        );
      } else {
        spans.addAll(
          _buildTextSpans(
            context,
            content,
            contentStyle,
            matchOffset + contentStart,
            composingRange,
            renderAgentMentions: false,
          ),
        );
      }
      spans.add(
        TextSpan(
          text: fullMatch.substring(contentEnd),
          style: _hiddenSyntaxStyle(inheritedStyle),
        ),
      );

      offset += nextMatch.end;
    }

    return spans;
  }

  List<InlineSpan> _buildTextSpans(
    BuildContext context,
    String source,
    TextStyle style,
    int sourceOffset,
    TextRange composingRange, {
    bool renderAgentMentions = true,
  }) {
    List<InlineSpan> buildTextSegment(String text, TextStyle segmentStyle) =>
        renderAgentMentions
        ? _buildAgentMentionSpans(context, text, segmentStyle)
        : [TextSpan(text: text, style: segmentStyle)];

    if (source.isEmpty) return const [];
    final localStart = (composingRange.start - sourceOffset)
        .clamp(0, source.length)
        .toInt();
    final localEnd = (composingRange.end - sourceOffset)
        .clamp(0, source.length)
        .toInt();
    if (!composingRange.isValid ||
        composingRange.isCollapsed ||
        localStart >= localEnd) {
      return buildTextSegment(source, style);
    }

    final composingDecorations = [
      if (style.decoration != null && style.decoration != TextDecoration.none)
        style.decoration!,
      TextDecoration.underline,
    ];
    final composingStyle = style.copyWith(
      decoration: TextDecoration.combine(composingDecorations),
    );
    return [
      if (localStart > 0)
        ...buildTextSegment(source.substring(0, localStart), style),
      TextSpan(
        text: source.substring(localStart, localEnd),
        style: composingStyle,
      ),
      if (localEnd < source.length)
        ...buildTextSegment(source.substring(localEnd), style),
    ];
  }

  List<InlineSpan> _buildAgentMentionSpans(
    BuildContext context,
    String source,
    TextStyle style,
  ) {
    if (_agentMentionNames.isEmpty) {
      return [TextSpan(text: source, style: style)];
    }

    final escapedNames = _agentMentionNames.toList()
      ..sort((a, b) => b.length.compareTo(a.length));
    final expression = RegExp(
      r'(^|\s)@(' +
          escapedNames.map(RegExp.escape).join('|') +
          r')(?=\s|[,.!?:;)\]}*_]|$)',
      caseSensitive: false,
      multiLine: true,
    );
    final spans = <InlineSpan>[];
    var offset = 0;
    for (final match in expression.allMatches(source)) {
      final prefix = match.group(1)!;
      if (match.start > offset) {
        spans.add(
          TextSpan(text: source.substring(offset, match.start), style: style),
        );
      }
      if (prefix.isNotEmpty) spans.add(TextSpan(text: prefix, style: style));

      final label = match.group(2)!;
      spans.add(
        WidgetSpan(
          alignment: PlaceholderAlignment.baseline,
          baseline: TextBaseline.alphabetic,
          child: _ComposerAgentMentionChip(label: label, textStyle: style),
        ),
      );
      // The visual chip replaces the `@` placeholder. Keep the label as
      // invisible source text so the text span still has one character per
      // source character, preserving native cursor and deletion behavior.
      spans.add(
        TextSpan(
          text: label,
          semanticsLabel: '',
          style: _hiddenMentionTextStyle(style),
        ),
      );
      offset = match.end;
    }
    if (offset < source.length) {
      spans.add(TextSpan(text: source.substring(offset), style: style));
    }
    return spans.isEmpty ? [TextSpan(text: source, style: style)] : spans;
  }

  TextStyle _hiddenMentionTextStyle(TextStyle inheritedStyle) =>
      inheritedStyle.copyWith(
        color: Colors.transparent,
        fontSize: 0.01,
        height: 0.01,
        letterSpacing: 0,
        decoration: TextDecoration.none,
      );

  (int, int) _contentBounds(String fullMatch, _MarkdownStyle markdownStyle) {
    final delimiterLength = switch (markdownStyle) {
      _MarkdownStyle.bold || _MarkdownStyle.strikethrough => 2,
      _MarkdownStyle.italic || _MarkdownStyle.inlineCode => 1,
      _MarkdownStyle.codeBlock => 3,
    };
    if (markdownStyle != _MarkdownStyle.codeBlock) {
      return (delimiterLength, fullMatch.length - delimiterLength);
    }

    final openingLength = fullMatch.startsWith('```\r\n')
        ? 5
        : fullMatch.startsWith('```\n')
        ? 4
        : delimiterLength;
    final closingLength = fullMatch.endsWith('\r\n```')
        ? 5
        : fullMatch.endsWith('\n```')
        ? 4
        : delimiterLength;
    return (openingLength, fullMatch.length - closingLength);
  }

  TextStyle _styleFor(
    BuildContext context,
    TextStyle inheritedStyle,
    _MarkdownStyle markdownStyle,
  ) {
    return switch (markdownStyle) {
      _MarkdownStyle.bold => inheritedStyle.merge(
        const TextStyle(fontWeight: FontWeight.w700),
      ),
      _MarkdownStyle.italic => inheritedStyle.merge(
        const TextStyle(fontStyle: FontStyle.italic),
      ),
      _MarkdownStyle.strikethrough => inheritedStyle.merge(
        const TextStyle(decoration: TextDecoration.lineThrough),
      ),
      _MarkdownStyle.inlineCode ||
      _MarkdownStyle.codeBlock => inheritedStyle.merge(
        TextStyle(
          fontFamily: 'GeistMono',
          color: context.colors.onSurface,
          backgroundColor: context.colors.surface,
        ),
      ),
    };
  }

  TextStyle _hiddenSyntaxStyle(TextStyle inheritedStyle) {
    return inheritedStyle.copyWith(
      color: Colors.transparent,
      fontSize: 0.01,
      height: 0.01,
      letterSpacing: 0,
      decoration: TextDecoration.none,
    );
  }
}

class _ComposerAgentMentionChip extends StatelessWidget {
  final String label;
  final TextStyle textStyle;

  const _ComposerAgentMentionChip({
    required this.label,
    required this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style = textStyle.copyWith(
      color: context.colors.primary,
      fontWeight: FontWeight.w500,
      height: 1,
    );
    final fontSize = style.fontSize ?? 16;

    return Semantics(
      label: 'Agent mention: $label',
      excludeSemantics: true,
      child: Container(
        key: const ValueKey('composer-agent-mention-chip'),
        padding: const EdgeInsets.fromLTRB(
          Grid.half,
          Grid.quarter + 1,
          Grid.half,
          Grid.quarter,
        ),
        decoration: BoxDecoration(
          // The composer surface is already tinted, so the body chip's
          // low-opacity fill disappears here. Keep the same chip geometry
          // while giving this editable token enough contrast to read as one.
          color: context.colors.primary.withValues(alpha: 0.16),
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(
            color: context.colors.primary.withValues(alpha: 0.12),
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            Icon(
              LucideIcons.bot,
              size: fontSize * 0.95,
              color: context.colors.primary,
            ),
            const SizedBox(width: Grid.quarter),
            Text(label, style: style),
          ],
        ),
      ),
    );
  }
}
