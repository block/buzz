part of '../message_content.dart';

String? _channelNameForId(Map<String, String> channels, String channelId) {
  for (final entry in channels.entries) {
    if (entry.value == channelId) return entry.key;
  }
  return null;
}

class _ChannelLinkMd extends InlineMd {
  final Map<String, String> channelNames;
  final void Function(String channelId)? onChannelTap;
  late final RegExp _exp = _buildPrefixPattern(
    prefix: '#',
    knownNames: channelNames.keys,
    genericTokenPattern: r'[A-Za-z0-9_][A-Za-z0-9_-]*',
  );

  _ChannelLinkMd({required this.channelNames, this.onChannelTap});

  @override
  RegExp get exp => _exp;

  @override
  InlineSpan span(
    BuildContext context,
    String text,
    final GptMarkdownConfig config,
  ) {
    final raw = exp.firstMatch(text.trim())?.group(0);
    if (raw == null) {
      return TextSpan(text: text, style: config.style);
    }

    final channelId = channelNames[raw.substring(1).toLowerCase()];
    final channelName = raw.substring(1);
    final opensChannel = channelId != null && onChannelTap != null;
    final child = _TokenPill(
      icon: LucideIcons.hash,
      interactive: opensChannel,
      semanticLabel: opensChannel
          ? 'Open channel $channelName'
          : 'Channel $channelName',
      text: channelName,
      textStyle: config.style?.copyWith(fontWeight: FontWeight.w500),
    );

    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: opensChannel
          ? GestureDetector(onTap: () => onChannelTap!(channelId), child: child)
          : child,
    );
  }
}

class _TokenPill extends StatelessWidget {
  final IconData? icon;
  final bool interactive;
  final String? semanticLabel;
  final String text;
  final TextStyle? textStyle;

  const _TokenPill({
    super.key,
    this.icon,
    this.interactive = false,
    this.semanticLabel,
    required this.text,
    this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style =
        textStyle?.copyWith(color: context.colors.primary) ??
        context.textTheme.bodyMedium?.copyWith(color: context.colors.primary);
    final fontSize = style?.fontSize ?? 16;
    final pill = Container(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
      decoration: BoxDecoration(
        color: context.colors.primary.withValues(alpha: 0.15),
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          if (icon != null) ...[
            Icon(icon, size: fontSize * 0.95, color: context.colors.primary),
            const SizedBox(width: Grid.quarter + 1),
          ],
          Text(text, style: style),
        ],
      ),
    );
    if (semanticLabel == null) return pill;
    return Semantics(
      label: semanticLabel,
      button: interactive,
      child: ExcludeSemantics(child: pill),
    );
  }
}

RegExp _buildPrefixPattern({
  required String prefix,
  required Iterable<String> knownNames,
  required String genericTokenPattern,
}) {
  final names =
      knownNames
          .map((name) => name.trim())
          .where((name) => name.isNotEmpty)
          .toSet()
          .toList()
        ..sort((a, b) => b.length.compareTo(a.length));

  final escapedPrefix = RegExp.escape(prefix);
  const leadingBoundary = r'(?<![\w./:-])';
  const trailingBoundary = r'(?=$|[\s,;.!?:)\]}])';

  if (names.isEmpty) {
    return RegExp(
      '$leadingBoundary$escapedPrefix(?:$genericTokenPattern)$trailingBoundary',
      caseSensitive: false,
      multiLine: true,
    );
  }

  final knownAlternatives = names.map(RegExp.escape).join('|');
  return RegExp(
    '$leadingBoundary$escapedPrefix(?:(?:$knownAlternatives)$trailingBoundary|(?:$genericTokenPattern)$trailingBoundary)',
    caseSensitive: false,
    multiLine: true,
  );
}
