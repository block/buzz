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
