part of '../emoji_picker.dart';

/// Shown above the grid when the community palette could not be refreshed.
///
/// The palette that was last loaded stays in the grid below. The message is a
/// live region and the Retry button takes its only label from its own text.
class _CustomEmojiRetryBanner extends HookConsumerWidget {
  const _CustomEmojiRetryBanner();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.colors;
    final retrying = useState(false);

    Future<void> retry() async {
      retrying.value = true;
      try {
        await ref.read(customEmojiPaletteProvider.notifier).refresh();
      } finally {
        if (context.mounted) retrying.value = false;
      }
    }

    return Padding(
      key: const ValueKey('custom-emoji-retry-banner'),
      padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              liveRegion: true,
              child: Text(
                'Couldn’t load custom emoji.',
                style: context.textTheme.bodySmall?.copyWith(
                  color: colors.onSurfaceVariant,
                ),
              ),
            ),
          ),
          TextButton(
            key: const ValueKey('custom-emoji-retry'),
            onPressed: retrying.value ? null : retry,
            child: const Text('Retry'),
          ),
        ],
      ),
    );
  }
}
