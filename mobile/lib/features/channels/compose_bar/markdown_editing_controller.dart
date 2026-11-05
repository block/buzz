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
  TextSpan? _cachedTextSpan;
  String? _cachedText;
  TextRange? _cachedComposingRange;
  TextStyle? _cachedBaseStyle;
  Color? _cachedOnSurface;
  Color? _cachedSurface;
  Map<String, String> _channelNames = const {};

  /// Source ranges rendered as atomic chips in [_chipRangesText], recorded
  /// while building the text span so deletion matches what is on screen.
  String? _chipRangesText;
  List<TextRange> _chipRanges = const [];
  final List<TextRange> _pendingChipRanges = [];

  /// Start of the link [buildTextSpan] last drew as raw text for the caret.
  int? _revealedLinkStart;

  /// Where the composer's Buzz links sit in [text] (URL only, without
  /// trailing punctuation), computed once per text.
  String? _linkRangesText;
  List<TextRange> _linkRanges = const [];

  static final _linkPattern = RegExp(
    r'''buzz://(?:message\?|channel/|(?:repo|pr|issue)\?)[^\s<>"']+''',
    caseSensitive: false,
  );
  static final _linkTrailingPunctuation = RegExp(r'[.,!?:;)\]}*]+$');

  /// Snaps a selection that would land inside a chip out to the chip's
  /// edges, so a chip is always wholly inside or outside the selection. A
  /// caret inside a chip moves to its nearer edge; at a link's end the link
  /// then shows as raw text.
  @override
  set value(TextEditingValue newValue) {
    super.value = _snapSelectionToChips(newValue);
    _recentValues
      ..remove(super.value)
      ..add(super.value);
    if (_recentValues.length > _recentValueLimit) {
      _recentValues.remove(_recentValues.first);
    }
  }

  /// Values this controller held recently, oldest first. Undo and redo
  /// restore one of these exactly, which tells them apart from a paste of a
  /// link.
  final _recentValues = <TextEditingValue>{};
  static const _recentValueLimit = 100;

  TextEditingValue _snapSelectionToChips(TextEditingValue next) {
    final selection = next.selection;
    if (_chipRanges.isEmpty ||
        next.text != _chipRangesText ||
        !selection.isValid ||
        (next.composing.isValid && !next.composing.isCollapsed)) {
      return next;
    }
    var start = selection.start;
    var end = selection.end;
    for (final chip in _chipRanges) {
      if (selection.isCollapsed) {
        if (chip.start < start && start < chip.end) {
          // A one-character step in from an edge is an arrow key, so keep
          // going and step over the chip; anything else (a tap) goes to the
          // nearer edge.
          final from = value.selection.isCollapsed
              ? value.selection.baseOffset
              : -1;
          final towardEnd = from == chip.start && start == chip.start + 1
              ? true
              : from == chip.end && start == chip.end - 1
              ? false
              : start - chip.start > chip.end - start;
          start = end = towardEnd ? chip.end : chip.start;
        }
        continue;
      }
      if (chip.start < start && start < chip.end) start = chip.start;
      if (chip.start < end && end < chip.end) end = chip.end;
    }
    if (start == selection.start && end == selection.end) return next;
    final snapped = selection.isCollapsed
        ? TextSelection.collapsed(offset: start)
        : selection.baseOffset <= selection.extentOffset
        ? TextSelection(baseOffset: start, extentOffset: end)
        : TextSelection(baseOffset: end, extentOffset: start);
    return next.copyWith(selection: snapped);
  }

  List<TextRange> _linkRangesIn(String source) {
    if (_linkRangesText == source) return _linkRanges;
    _linkRangesText = source;
    return _linkRanges = List.unmodifiable([
      for (final match in _linkPattern.allMatches(source))
        TextRange(
          start: match.start,
          end:
              match.end -
              (_linkTrailingPunctuation
                      .firstMatch(match.group(0)!)
                      ?.group(0)
                      ?.length ??
                  0),
        ),
    ]);
  }

  /// The link shown as raw text because the selection sits in it: a caret
  /// at its end (or a selection inside it, entered from there). A caret at
  /// its start leaves it a chip.
  TextRange? _revealedLinkFor(String source, TextSelection selection) {
    if (!selection.isValid) return null;
    for (final link in _linkRangesIn(source)) {
      if (selection.start > link.start && selection.end <= link.end) {
        return link;
      }
    }
    return null;
  }

  void setChannelNames(Map<String, String> names) {
    if (mapEquals(_channelNames, names)) return;
    _channelNames = Map.unmodifiable(names);
    _cachedTextSpan = null;
    notifyListeners();
  }

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
    _cachedTextSpan = null;
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
    final colors = context.colors;
    final revealedLinkStart = _revealedLinkFor(text, selection)?.start;
    if (_cachedTextSpan case final cached?
        when _cachedText == text &&
            _revealedLinkStart == revealedLinkStart &&
            _cachedComposingRange == composingRange &&
            _cachedBaseStyle == baseStyle &&
            _cachedOnSurface == colors.onSurface &&
            _cachedSurface == colors.surface) {
      return cached;
    }
    _pendingChipRanges.clear();
    _revealedLinkStart = revealedLinkStart;
    final span = TextSpan(
      style: baseStyle,
      children: _buildMarkdownSpans(
        context,
        text,
        baseStyle,
        sourceOffset: 0,
        composingRange: composingRange,
      ),
    );
    _cachedText = text;
    _cachedComposingRange = composingRange;
    _cachedBaseStyle = baseStyle;
    _cachedOnSurface = colors.onSurface;
    _cachedSurface = colors.surface;
    _cachedTextSpan = span;
    _chipRangesText = text;
    _chipRanges = List.unmodifiable(_pendingChipRanges);
    return span;
  }

  /// Applies the composer's chip editing rules to a user edit.
  TextEditingValue formatUserEdit(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) => _spaceAfterPastedLink(oldValue, expandChipDeletion(oldValue, newValue));

  /// Adds a space after a link pasted on its own, so it becomes a chip
  /// straight away. Pasted text with anything besides one link, and Undo or
  /// Redo back to an earlier value, is unchanged.
  TextEditingValue _spaceAfterPastedLink(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) {
    final oldSelection = oldValue.selection;
    if (!oldSelection.isValid ||
        (newValue.composing.isValid && !newValue.composing.isCollapsed) ||
        _recentValues.contains(newValue)) {
      return newValue;
    }
    final oldText = oldValue.text;
    final newText = newValue.text;
    final start = oldSelection.start;
    final end = oldSelection.end;
    final insertedEnd = newText.length - (oldText.length - end);
    if (insertedEnd - start < 2 ||
        !newText.startsWith(oldText.substring(0, start)) ||
        !newText.endsWith(oldText.substring(end))) {
      return newValue;
    }
    final inserted = newText.substring(start, insertedEnd);
    final match = _linkPattern.matchAsPrefix(inserted);
    if (match == null ||
        match.end != inserted.length ||
        _linkTrailingPunctuation.hasMatch(inserted) ||
        (insertedEnd < newText.length && newText[insertedEnd].trim().isEmpty)) {
      return newValue;
    }
    return TextEditingValue(
      text: newText.replaceRange(insertedEnd, insertedEnd, ' '),
      selection: TextSelection.collapsed(offset: insertedEnd + 1),
    );
  }

  /// Widens a backspace or forward delete that removes part of a chip's
  /// hidden source so it removes the whole chip. Each chip draws its whole
  /// URL or `@label` as one glyph, so backspace right after it would
  /// otherwise remove one invisible character of the source and leave a
  /// broken token behind. Edits over a selection replace exactly that
  /// selection, so Cut never removes more than it copies.
  TextEditingValue expandChipDeletion(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) {
    final oldSelection = oldValue.selection;
    if (_chipRanges.isEmpty ||
        _chipRangesText != oldValue.text ||
        !oldSelection.isValid ||
        !oldSelection.isCollapsed ||
        (oldValue.composing.isValid && !oldValue.composing.isCollapsed) ||
        (newValue.composing.isValid && !newValue.composing.isCollapsed)) {
      return newValue;
    }
    final newSelection = newValue.selection;
    if (!newSelection.isValid || !newSelection.isCollapsed) return newValue;
    final deleted = _caretDeletion(
      oldValue.text,
      newValue.text,
      oldSelection.baseOffset,
      newSelection.baseOffset,
    );
    if (deleted == null) return newValue;
    // A link the caret has revealed edits as raw text, even before the next
    // frame drops its chip.
    final revealed = _revealedLinkFor(oldValue.text, oldSelection);
    var expandedStart = deleted.start;
    var expandedEnd = deleted.end;
    for (final chip in _chipRanges) {
      if (chip == revealed) continue;
      // Only a deletion inside one chip widens. A run reaching past the chip
      // is not a key press (Undo can restore a shorter link this way), and
      // leaves the rest as visible text.
      if (chip.start <= deleted.start && deleted.end <= chip.end) {
        expandedStart = chip.start;
        expandedEnd = chip.end;
      }
    }
    if (expandedStart == deleted.start && expandedEnd == deleted.end) {
      return newValue;
    }
    return TextEditingValue(
      text: oldValue.text.replaceRange(expandedStart, expandedEnd, ''),
      selection: TextSelection.collapsed(offset: expandedStart),
    );
  }

  /// The run a collapsed caret deleted: one ending at it (backspace, which
  /// moves the caret back) or starting at it (forward delete, which leaves
  /// the caret in place), or null for any other edit. The caret's movement
  /// tells the two apart when repeated characters make the text ambiguous.
  static TextRange? _caretDeletion(
    String oldText,
    String newText,
    int caret,
    int newCaret,
  ) {
    final removed = oldText.length - newText.length;
    if (removed <= 0) return null;
    bool removes(int start, int end) =>
        start >= 0 &&
        end <= oldText.length &&
        newText.startsWith(oldText.substring(0, start)) &&
        newText.endsWith(oldText.substring(end));
    if (newCaret == caret - removed && removes(caret - removed, caret)) {
      return TextRange(start: caret - removed, end: caret);
    }
    if (newCaret == caret && removes(caret, caret + removed)) {
      return TextRange(start: caret, end: caret + removed);
    }
    return null;
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
    List<InlineSpan> buildTextSegment(
      String text,
      TextStyle segmentStyle,
      int segmentOffset,
    ) => renderAgentMentions
        ? _buildAgentMentionSpans(
            context,
            text,
            segmentStyle,
            sourceOffset: segmentOffset,
          )
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
      return buildTextSegment(source, style, sourceOffset);
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
        ...buildTextSegment(
          source.substring(0, localStart),
          style,
          sourceOffset,
        ),
      TextSpan(
        text: source.substring(localStart, localEnd),
        style: composingStyle,
      ),
      if (localEnd < source.length)
        ...buildTextSegment(
          source.substring(localEnd),
          style,
          sourceOffset + localEnd,
        ),
    ];
  }

  List<InlineSpan> _buildAgentMentionSpans(
    BuildContext context,
    String source,
    TextStyle style, {
    required int sourceOffset,
    bool renderLinks = true,
  }) {
    final tokenSpans = renderLinks
        ? _buildComposerTokenSpans(context, source, style, sourceOffset)
        : null;
    if (tokenSpans != null) return tokenSpans;
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
      if (RegExp(r'\([0-9a-f]{64}\)').hasMatch(label)) {
        spans.add(
          TextSpan(
            text: '@$label',
            style: style.copyWith(color: context.colors.primary),
          ),
        );
        offset = match.end;
        continue;
      }
      spans.add(
        WidgetSpan(
          alignment: PlaceholderAlignment.baseline,
          baseline: TextBaseline.alphabetic,
          child: _ComposerAgentMentionChip(label: label, textStyle: style),
        ),
      );
      _pendingChipRanges.add(
        TextRange(
          start: sourceOffset + match.start + prefix.length,
          end: sourceOffset + match.end,
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

  List<InlineSpan>? _buildComposerTokenSpans(
    BuildContext context,
    String source,
    TextStyle style,
    int sourceOffset,
  ) {
    final matches = _linkPattern.allMatches(source).toList();
    if (matches.isEmpty) return null;

    final spans = <InlineSpan>[];
    var offset = 0;
    for (final match in matches) {
      if (match.start > offset) {
        spans.addAll(
          _buildAgentMentionSpans(
            context,
            source.substring(offset, match.start),
            style,
            sourceOffset: sourceOffset + offset,
            renderLinks: false,
          ),
        );
      }
      final raw = match.group(0)!;
      final url = raw.replaceFirst(_linkTrailingPunctuation, '');
      final trailing = raw.substring(url.length);
      final linkStart = sourceOffset + match.start;
      // A link shows as raw text while the caret sits at its end, so it can
      // be edited. The match runs to whitespace or the end of the text, so
      // everywhere else a boundary follows it and it is a chip.
      final presentation = linkStart == _revealedLinkStart
          ? null
          : _composerLinkPresentation(url);
      if (presentation == null) {
        spans.add(TextSpan(text: raw, style: style));
      } else {
        spans.add(
          WidgetSpan(
            alignment: PlaceholderAlignment.baseline,
            baseline: TextBaseline.alphabetic,
            child: _ComposerBuzzLinkChip(
              icon: presentation.$1,
              label: presentation.$2,
              semanticLabel: presentation.$3,
              textStyle: style,
            ),
          ),
        );
        _pendingChipRanges.add(
          TextRange(
            start: sourceOffset + match.start,
            end: sourceOffset + match.start + url.length,
          ),
        );
        // Preserve one source character per input character for native cursor
        // movement while the visible atomic chip replaces the URL.
        spans.add(
          TextSpan(
            text: url.substring(1),
            semanticsLabel: '',
            style: _hiddenMentionTextStyle(style),
          ),
        );
        if (trailing.isNotEmpty) {
          spans.add(TextSpan(text: trailing, style: style));
        }
      }
      offset = match.end;
    }
    if (offset < source.length) {
      spans.addAll(
        _buildAgentMentionSpans(
          context,
          source.substring(offset),
          style,
          sourceOffset: sourceOffset + offset,
          renderLinks: false,
        ),
      );
    }
    return spans;
  }

  (IconData, String, String)? _composerLinkPresentation(String raw) {
    final uri = Uri.tryParse(raw);
    if (uri == null) return null;
    final link = parseBuzzDeepLink(uri) ?? parseEntityDeepLink(uri);
    return switch (link) {
      ChannelDeepLink(:final channelId) => (
        BuzzIcons.hash,
        _resolvedChannelName(channelId),
        'Channel ${_resolvedChannelName(channelId)}',
      ),
      MessageDeepLink(:final channelId, :final messageId) => (
        BuzzIcons.messageSquare,
        '${_resolvedChannelName(channelId)} · ${messageId.substring(0, 8)}',
        'Message ${messageId.substring(0, 8)} in channel ${_resolvedChannelName(channelId)}',
      ),
      EntityDeepLink(:final type, :final repository, :final eventId) => (
        switch (type) {
          'repo' => BuzzIcons.folderGit2,
          'pr' => BuzzIcons.gitPullRequest,
          _ => BuzzIcons.circleDot,
        },
        type == 'repo'
            ? repository
            : '$repository · ${eventId!.substring(0, 8)}',
        switch (type) {
          'repo' => 'Repository $repository',
          'pr' =>
            'Pull request ${eventId!.substring(0, 8)} in repository $repository',
          _ => 'Issue ${eventId!.substring(0, 8)} in repository $repository',
        },
      ),
      _ => null,
    };
  }

  String _resolvedChannelName(String channelId) {
    for (final entry in _channelNames.entries) {
      if (entry.value == channelId) return entry.key;
    }
    return channelId.substring(0, math.min(8, channelId.length));
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

class _ComposerBuzzLinkChip extends StatelessWidget {
  final IconData icon;
  final String label;
  final String semanticLabel;
  final TextStyle textStyle;

  const _ComposerBuzzLinkChip({
    required this.icon,
    required this.label,
    required this.semanticLabel,
    required this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style = textStyle.copyWith(
      color: context.colors.primary,
      fontWeight: FontWeight.w600,
      height: 1,
    );
    final fontSize = style.fontSize ?? 16;
    return Semantics(
      label: semanticLabel,
      excludeSemantics: true,
      child: Container(
        key: ValueKey('composer-buzz-link-chip:$label'),
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        decoration: BoxDecoration(
          color: context.colors.primary.withValues(alpha: 0.16),
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(
            color: context.colors.primary.withValues(alpha: 0.12),
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: fontSize * 0.95, color: context.colors.primary),
            const SizedBox(width: Grid.quarter + 1),
            Text(label, style: style),
          ],
        ),
      ),
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
              BuzzIcons.bot,
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

/// Routes user edits through [_MarkdownEditingController.formatUserEdit] so
/// composer chips follow their editing rules.
class _ComposerChipEditingFormatter extends TextInputFormatter {
  final _MarkdownEditingController controller;

  _ComposerChipEditingFormatter(this.controller);

  @override
  TextEditingValue formatEditUpdate(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) => controller.formatUserEdit(oldValue, newValue);
}
