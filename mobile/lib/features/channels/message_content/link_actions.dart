part of '../message_content.dart';

/// Wraps a rendered link so a hold opens [_showLinkActions] instead of the
/// surrounding message's menu.
///
/// The recognizer shares [iosMessageLongPressDuration] with the message row.
/// Pointer events reach the deepest recognizer first, so at equal deadlines
/// the link's timer fires and wins the arena before the message row's does,
/// the same way reaction pills claim their own hold.
class _LinkLongPress extends StatelessWidget {
  final String url;
  final VoidCallback onOpen;
  final Widget child;

  const _LinkLongPress({
    required this.url,
    required this.onOpen,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    void open() => _showLinkActions(context, url: url, onOpen: onOpen);

    return Semantics(
      onLongPress: open,
      child: RawGestureDetector(
        gestures: {
          LongPressGestureRecognizer:
              GestureRecognizerFactoryWithHandlers<LongPressGestureRecognizer>(
                () => LongPressGestureRecognizer(
                  duration: defaultTargetPlatform == TargetPlatform.iOS
                      ? iosMessageLongPressDuration
                      : null,
                ),
                (recognizer) => recognizer.onLongPress = open,
              ),
        },
        child: child,
      ),
    );
  }
}

/// Link-specific actions: open, copy and share the destination.
void _showLinkActions(
  BuildContext context, {
  required String url,
  required VoidCallback onOpen,
}) {
  unawaited(HapticFeedback.mediumImpact());
  final uri = Uri.tryParse(url);
  final shareable =
      uri != null && (uri.scheme == 'http' || uri.scheme == 'https');
  unawaited(
    showBuzzModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      title: 'Link',
      builder: (sheetContext) => SafeArea(
        child: IconTheme.merge(
          data: const IconThemeData(size: 22),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.only(bottom: Grid.xs),
                  child: Text(
                    url,
                    maxLines: 3,
                    overflow: TextOverflow.ellipsis,
                    style: sheetContext.textTheme.bodySmall?.copyWith(
                      color: sheetContext.colors.onSurfaceVariant,
                    ),
                  ),
                ),
                SheetActionSection(
                  children: [
                    ListTile(
                      leading: const Icon(BuzzIcons.externalLink),
                      title: const Text('Open link'),
                      onTap: () {
                        Navigator.of(sheetContext).pop();
                        onOpen();
                      },
                    ),
                    ListTile(
                      leading: const Icon(BuzzIcons.copy),
                      title: const Text('Copy link'),
                      onTap: () {
                        Navigator.of(sheetContext).pop();
                        unawaited(
                          copyToClipboard(context, url, message: 'Link copied'),
                        );
                      },
                    ),
                    if (shareable)
                      ListTile(
                        leading: const Icon(BuzzIcons.share2),
                        title: const Text('Share link'),
                        onTap: () {
                          final renderBox =
                              context.findRenderObject() as RenderBox?;
                          final shareOrigin = renderBox == null
                              ? null
                              : renderBox.localToGlobal(Offset.zero) &
                                    renderBox.size;
                          Navigator.of(sheetContext).pop();
                          unawaited(
                            SharePlus.instance.share(
                              ShareParams(
                                uri: uri,
                                sharePositionOrigin: shareOrigin,
                              ),
                            ),
                          );
                        },
                      ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
}
