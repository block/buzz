part of '../message_actions.dart';

class _QuickReactionRow extends ConsumerWidget {
  final TimelineMessage message;
  final List<TimelineReaction> reactions;
  final ValueChanged<String>? onToggleReaction;

  /// The sheet's context, popped before the reaction fires.
  final BuildContext sheetContext;

  /// The page navigator's context. The picker opened from "+" outlives both
  /// this sheet and the long-pressed row, which a new message can rebuild away.
  final BuildContext pickerContext;

  /// The page's container. Reactions outlive this sheet and the long-pressed
  /// row, so they must not read through either one's disposed ref.
  final ProviderContainer pageContainer;

  /// Whether the community active when the sheet opened is still active. A
  /// notification can switch community while the sheet or its picker is up,
  /// and a reaction must not then go out as the other account.
  final bool Function() stillCurrent;

  const _QuickReactionRow({
    required this.message,
    required this.reactions,
    this.onToggleReaction,
    required this.sheetContext,
    required this.pickerContext,
    required this.pageContainer,
    required this.stillCurrent,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final dataset = ref.watch(emojiDatasetOrEmptyProvider);
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
      if (!stillCurrent()) return;
      if (reactions.any((r) => r.emoji == value && r.reactedByCurrentUser)) {
        if (onToggleReaction != null) {
          onToggleReaction!(value);
        } else {
          toggleReaction(pickerContext, pageContainer, message, value);
        }
        return;
      }
      // The generic picker is also used for composing and statuses. Record
      // recency here, at the reaction call site, so only reactions drive the
      // quick-reaction row.
      pageContainer.read(recentEmojiProvider.notifier).record(value);
      // The sheet is on its way out, so the burst can't come from this tile —
      // hand it to the pill that's about to appear in the timeline.
      armReactionBurst(pageContainer, message, value);
      if (onToggleReaction != null) {
        onToggleReaction!(value);
      } else {
        reportReactionFailure(
          pickerContext,
          pageContainer
              .read(channelActionsProvider)
              .addReaction(message.id, value, targetAuthor: message.pubkey),
          adding: true,
        );
      }
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
                label: dataset.displayName(emoji[index]),
                selected: reactions.any(
                  (r) => r.emoji == emoji[index] && r.reactedByCurrentUser,
                ),
                onTap: () {
                  if (!_selectMessageAction(context)) return;
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
              label: 'Add reaction',
              onTap: () {
                if (!_selectMessageAction(context)) return;
                showEmojiPicker(
                  context: pickerContext,
                  onSelect: (value) {
                    react(value);
                  },
                );
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
