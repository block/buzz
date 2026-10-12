part of '../message_actions.dart';

class _QuickReactionRow extends ConsumerWidget {
  final TimelineMessage message;

  /// The sheet's context, popped before the reaction fires.
  final BuildContext sheetContext;

  /// The long-pressed message's page context — survives the sheet pop, so the
  /// picker opened from "+" isn't torn down with the sheet.
  final BuildContext pageContext;

  /// The long-pressed message's page ref. The picker callback outlives this
  /// bottom sheet, so it must not read through the sheet's disposed ref.
  final WidgetRef pageRef;

  const _QuickReactionRow({
    required this.message,
    required this.sheetContext,
    required this.pageContext,
    required this.pageRef,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final customEmoji = ref.watch(customEmojiListProvider);
    final emoji = quickReactionEmoji(
      ref.watch(recentEmojiProvider),
      customShortcodes: {
        for (final entry in customEmoji) entry.shortcode.toLowerCase(),
      },
    );
    final customByShortcode = {
      for (final entry in customEmoji) entry.shortcode.toLowerCase(): entry,
    };

    void react(String value) {
      // The generic picker is also used for composing and statuses. Record
      // recency here, at the reaction call site, so only reactions drive the
      // quick-reaction row.
      pageRef.read(recentEmojiProvider.notifier).record(value);
      // The sheet is on its way out, so the burst can't come from this tile —
      // hand it to the pill that's about to appear in the timeline.
      armReactionBurst(pageRef, message, value);
      pageRef.read(channelActionsProvider).addReaction(message.id, value);
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        const desiredCircleSize = 52.0;
        const minimumCircleSize = 44.0;
        final itemCount = emoji.length + 1;
        final gapCount = itemCount - 1;
        final circleSize =
            ((constraints.maxWidth - (Grid.twelve * gapCount)) / itemCount)
                .clamp(minimumCircleSize, desiredCircleSize)
                .toDouble();
        final gap =
            ((constraints.maxWidth - (circleSize * itemCount)) / gapCount)
                .clamp(0.0, Grid.twelve)
                .toDouble();
        final circles = <Widget>[
          for (var index = 0; index < emoji.length; index++)
            KeyedSubtree(
              key: ValueKey('quick-reaction-${emoji[index]}'),
              child: _QuickReactionCircle(
                size: circleSize,
                onTap: () {
                  Navigator.of(sheetContext).pop();
                  react(emoji[index]);
                },
                child: _QuickReactionGlyph(
                  value: emoji[index],
                  customByShortcode: customByShortcode,
                ),
              ),
            ),
          KeyedSubtree(
            key: const ValueKey('quick-reaction-more'),
            child: _QuickReactionCircle(
              size: circleSize,
              onTap: () {
                Navigator.of(sheetContext).pop();
                showEmojiPicker(context: pageContext, onSelect: react);
              },
              child: Icon(
                BuzzIcons.plus,
                size: 24,
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        ];

        return Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            for (var index = 0; index < circles.length; index++) ...[
              circles[index],
              if (index < circles.length - 1) SizedBox(width: gap),
            ],
          ],
        );
      },
    );
  }
}
