part of '../message_content.dart';

/// A video bubble. It shows the NIP-71 poster when the event has one;
/// otherwise a static placeholder, so scrolling never starts a video decoder.
class _MessageVideoPreview extends StatelessWidget {
  final String url;
  final ImetaEntry? imeta;
  final VoidCallback? onReply;

  const _MessageVideoPreview({
    required this.url,
    required this.imeta,
    required this.onReply,
  });

  @override
  Widget build(BuildContext context) {
    final aspectRatio = messageVideoAspectRatio(imeta?.aspectRatio);
    final posterUrl = imeta?.posterUrl;

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: GestureDetector(
        onTap: () => openVideoViewer(
          context,
          videoUrl: url,
          posterUrl: posterUrl,
          onReply: onReply,
        ),
        child: _MessageMediaPreviewFrame(
          previewKey: ValueKey('message-media-video-preview:$url'),
          backgroundColor: Colors.black,
          child: AspectRatio(
            aspectRatio: aspectRatio.toDouble(),
            child: Stack(
              fit: StackFit.expand,
              children: [
                if (posterUrl != null)
                  MediaImage(
                    url: posterUrl,
                    fit: BoxFit.cover,
                    frameBuilder: (context, child, frame, synchronous) =>
                        frame != null || synchronous
                        ? child
                        : const MediaLoadingPlaceholder(label: 'Loading video'),
                    errorBuilder: (_, _, _) => const _MediaPreviewFallback(
                      icon: BuzzIcons.video,
                      label: 'Video preview unavailable',
                    ),
                  )
                else
                  const _MediaPreviewFallback(
                    icon: BuzzIcons.video,
                    label: 'Video attachment',
                  ),
                const ColoredBox(color: Color.fromRGBO(0, 0, 0, 0.28)),
                Center(
                  child: Container(
                    width: 52,
                    height: 52,
                    decoration: const BoxDecoration(
                      color: Color.fromRGBO(0, 0, 0, 0.6),
                      shape: BoxShape.circle,
                    ),
                    child: const Icon(
                      BuzzIcons.play,
                      color: Colors.white,
                      size: 24,
                    ),
                  ),
                ),
                Positioned(
                  left: Grid.xxs,
                  right: Grid.xxs,
                  bottom: Grid.xxs,
                  child: Text(
                    'Video',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: Colors.white,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
