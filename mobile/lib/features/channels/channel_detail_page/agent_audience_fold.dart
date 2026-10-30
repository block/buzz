part of '../channel_detail_page.dart';

/// Collapses an agent-to-agent message to one line until tapped.
class _AgentAudienceFold extends HookConsumerWidget {
  final TimelineMessage message;
  final String channelId;
  final Widget child;

  const _AgentAudienceFold({
    required this.message,
    required this.channelId,
    required this.child,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final expanded = useState(false);
    final author = message.pubkey.toLowerCase();
    final recipients = [
      for (final pubkey in message.mentionPubkeys)
        if (pubkey.toLowerCase() != author) pubkey.toLowerCase(),
    ];
    final labels = watchChannelIdentityLabels(ref, channelId, [
      author,
      ...recipients,
    ]);
    if (expanded.value) {
      return GestureDetector(onTap: () => expanded.value = false, child: child);
    }
    var preview = message.content.trimLeft();
    for (final label in [for (final r in recipients) '@${labels[r]}']) {
      if (preview.startsWith(label)) {
        preview = preview.substring(label.length).trimLeft();
      }
    }
    final metadata = context.textTheme.labelSmall?.copyWith(
      color: context.colors.onSurfaceVariant,
    );
    return InkWell(
      key: ValueKey('agent-audience-${message.id}'),
      onTap: () => expanded.value = true,
      borderRadius: BorderRadius.circular(Radii.md),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        child: Row(
          children: [
            Flexible(
              child: Text(
                [
                  labels[author],
                  if (recipients.isNotEmpty)
                    recipients.map((r) => labels[r]).join(', '),
                ].join(' → '),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: metadata?.copyWith(fontWeight: FontWeight.w600),
              ),
            ),
            const SizedBox(width: Grid.xs),
            Expanded(
              child: Text(
                preview,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: metadata,
              ),
            ),
            const SizedBox(width: Grid.xs),
            Text(formatMessageTime(message.createdAt), style: metadata),
          ],
        ),
      ),
    );
  }
}
