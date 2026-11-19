part of '../message_content.dart';

/// A video bubble. It shows the NIP-71 poster when the event has one;
/// otherwise a static placeholder, so scrolling never starts a video decoder.
class _MessageVideoPreview extends HookConsumerWidget {
  final String url;
  final ImetaEntry? imeta;
  final VoidCallback? onReply;

  const _MessageVideoPreview({
    required this.url,
    required this.imeta,
    required this.onReply,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sourceKey = useMemoized(GlobalKey.new);
    final opening = useRef(false);
    final aspectRatio = messageVideoAspectRatio(imeta?.aspectRatio);
    final posterUrl = imeta?.posterUrl;

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: GestureDetector(
        onTap: () async {
          if (opening.value) return;
          opening.value = true;
          NativeVideoFiles? files;
          try {
            if (NativePhotoZoomPreview.enabled) {
              await WidgetsBinding.instance.endOfFrame;
              if (!context.mounted) return;
              final auth = ref.read(mediaGetAuthServiceProvider);
              final client = ref.read(mediaHttpClientProvider);
              files = NativeVideoFiles(auth: auth, client: client, index: 0);
              final fallback = await NativePhotoZoomPreview.open(
                context: context,
                sourceKeys: [sourceKey],
                previewProviders: [
                  if (posterUrl != null)
                    MediaImageProvider(
                      url: posterUrl,
                      auth: auth,
                      client: client,
                    )
                  else
                    null,
                ],
                providers: [
                  MediaImageProvider(
                    url: posterUrl ?? url,
                    auth: auth,
                    client: client,
                  ),
                ],
                urls: [url],
                labels: [imeta?.alt ?? 'Video'],
                initialIndex: 0,
                videoIndices: {0},
                loadVideo: (_) => files!.load(0, url),
                onSelect: (_) async {},
                onReply: onReply,
              );
              if (!fallback || !context.mounted) return;
            }
            openVideoViewer(
              context,
              videoUrl: url,
              posterUrl: posterUrl,
              onReply: onReply,
            );
          } finally {
            await files?.dispose();
            opening.value = false;
          }
        },
        child: RepaintBoundary(
          key: sourceKey,
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
                          : const MediaLoadingPlaceholder(
                              label: 'Loading video',
                            ),
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
      ),
    );
  }
}
