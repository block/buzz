part of '../message_content.dart';

String? _channelNameForId(Map<String, String> channels, String channelId) {
  for (final entry in channels.entries) {
    if (entry.value == channelId) return entry.key;
  }
  return null;
}

/// `#channel` tokens. The default scopes keep this out of link labels: its
/// [WidgetSpan] nested inside the link's own placeholder does not paint on
/// iOS, so an authored `[#channel](url)` would render as nothing. Link
/// resolution wins over token detection inside a label.
InlinePattern _channelLinkPattern({
  required Map<String, String> channelNames,
  void Function(String channelId)? onChannelTap,
}) => InlinePattern.prefixed(
  prefix: '#',
  knownNames: channelNames.keys,
  genericTokenPattern: r'[A-Za-z0-9_][A-Za-z0-9_-]*',
  builder: (context, match, style) {
    final channelName = match.group(0)!.substring(1);
    final channelId = channelNames[channelName.toLowerCase()];
    final opensChannel = channelId != null && onChannelTap != null;
    final child = _TokenPill(
      icon: BuzzIcons.hash,
      interactive: opensChannel,
      semanticLabel: opensChannel
          ? 'Open channel $channelName'
          : 'Channel $channelName',
      text: channelName,
      textStyle: style.copyWith(fontWeight: FontWeight.w500),
    );

    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: opensChannel
          ? GestureDetector(onTap: () => onChannelTap(channelId), child: child)
          : child,
    );
  },
);

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
