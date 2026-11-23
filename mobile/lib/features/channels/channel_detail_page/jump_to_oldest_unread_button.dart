part of '../channel_detail_page.dart';

class _JumpToOldestUnreadButton extends StatelessWidget {
  const _JumpToOldestUnreadButton({required this.onPressed});

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => IconButton.filled(
    key: const ValueKey('channel-jump-to-oldest-unread'),
    onPressed: onPressed,
    tooltip: 'Jump to oldest unread message',
    style: IconButton.styleFrom(
      backgroundColor: context.controlSurface,
      foregroundColor: context.colors.onPrimaryContainer,
    ),
    icon: const Icon(BuzzIcons.chevronUp, size: 20),
  );
}
