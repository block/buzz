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
            content,
            contentStyle,
            matchOffset + contentStart,
            composingRange,
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
    String source,
    TextStyle style,
    int sourceOffset,
    TextRange composingRange,
  ) {
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
      return [TextSpan(text: source, style: style)];
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
        TextSpan(text: source.substring(0, localStart), style: style),
      TextSpan(
        text: source.substring(localStart, localEnd),
        style: composingStyle,
      ),
      if (localEnd < source.length)
        TextSpan(text: source.substring(localEnd), style: style),
    ];
  }

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
