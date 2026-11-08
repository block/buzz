part of 'me_page.dart';

class _MeEmptyState extends StatelessWidget {
  const _MeEmptyState({required this.onNewChat});

  final VoidCallback? onNewChat;

  @override
  Widget build(BuildContext context) {
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final button = TextButton(
      onPressed: onNewChat,
      style: TextButton.styleFrom(
        foregroundColor: context.colors.onSurface,
        backgroundColor: ios ? Colors.transparent : context.controlSurface,
        minimumSize: const Size(112, 48),
      ),
      child: const Text('New chat'),
    );
    return Padding(
      padding: const EdgeInsets.all(Grid.gutter),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: Grid.xxl,
            height: Grid.xxl,
            decoration: BoxDecoration(
              color: context.controlSurface,
              shape: BoxShape.circle,
            ),
            child: const Icon(BuzzIcons.messageSquare, size: 40),
          ),
          const SizedBox(height: Grid.gutter),
          Text(
            'A place to work with your agents',
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: Grid.gutter),
          if (ios)
            ConcentricSheetSurface(
              enabled: true,
              usesGlass: true,
              color: Colors.transparent,
              providesSheetSurface: false,
              padding: EdgeInsets.zero,
              minimumRadius: Radii.full,
              contentClipRadius: Radii.full,
              child: button,
            )
          else
            button,
        ],
      ),
    );
  }
}
