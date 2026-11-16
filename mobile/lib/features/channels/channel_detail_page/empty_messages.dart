part of '../channel_detail_page.dart';

class _EmptyMessages extends StatelessWidget {
  const _EmptyMessages();

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            BuzzIcons.messageSquare,
            size: Grid.xl,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            'No messages yet',
            style: context.textTheme.bodyLarge?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.half),
          Text(
            'Be the first to say something!',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

Widget _welcomeMessages(
  BuildContext context, {
  required double appBarTitleContentHeight,
  required double composerBottomInset,
  required List<Widget> children,
}) => SingleChildScrollView(
  padding: EdgeInsets.fromLTRB(
    Grid.gutter,
    _conversationHeaderHeight(
          context,
          titleContentHeight: appBarTitleContentHeight,
        ) +
        Grid.md,
    Grid.gutter,
    composerBottomInset + Grid.md,
  ),
  child: Column(children: children),
);

// Keep the empty, welcome and typing-only states together.
extension on _MessageList {
  Widget _buildEmptyTimeline(
    BuildContext context,
    Widget? hiddenHistory,
    List<TypingEntry> displayedTyping,
    Set<String> workingAgents,
  ) {
    if (welcomeAgent == null &&
        (displayedTyping.isNotEmpty || TypingMotionPreview.enabled)) {
      return Align(
        alignment: Alignment.bottomCenter,
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            Grid.gutter,
            0,
            Grid.gutter,
            composerBottomInset,
          ),
          child: _aboveHiddenHistory(
            hiddenHistory,
            TypingMotionPreview.enabled
                ? TypingMotionPreview(channelId: channelId)
                : InlineAgentTyping(
                    channelId: channelId,
                    entries: displayedTyping,
                    knownAgents: workingAgents,
                  ),
          ),
        ),
      );
    }
    if (welcomeAgent != null) {
      return _welcomeMessages(
        context,
        appBarTitleContentHeight: appBarTitleContentHeight,
        composerBottomInset: composerBottomInset,
        children: [
          BestieWelcome(agent: welcomeAgent!),
          ?hiddenHistory,
          if (displayedTyping.isNotEmpty)
            InlineAgentTyping(
              channelId: channelId,
              entries: displayedTyping,
              knownAgents: workingAgents,
            ),
        ],
      );
    }
    return hiddenHistory ?? const _EmptyMessages();
  }
}
