part of '../message_content.dart';

const _messageMediaCarouselHeight = 220.0;

@immutable
class _MessageGalleryItem {
  final String url;
  final String semanticLabel;
  final double? aspectRatio;

  const _MessageGalleryItem({
    required this.url,
    required this.semanticLabel,
    required this.aspectRatio,
  });
}

@immutable
class _TrailingImageGallery {
  final String content;
  final List<_MessageGalleryItem> items;

  const _TrailingImageGallery({required this.content, required this.items});
}

class _MessageGalleryPrecache extends HookWidget {
  final List<ImageProvider<Object>> providers;
  final int focusedIndex;

  const _MessageGalleryPrecache({
    required this.providers,
    required this.focusedIndex,
  });

  @override
  Widget build(BuildContext context) {
    final providerSignature = Object.hashAll(providers);
    useEffect(() {
      var cancelled = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (cancelled || !context.mounted) {
          return;
        }
        for (var index = focusedIndex - 2; index <= focusedIndex + 2; index++) {
          if (index < 0 || index >= providers.length) {
            continue;
          }
          unawaited(
            precacheImage(providers[index], context, onError: (_, _) {}),
          );
        }
      });
      return () => cancelled = true;
    }, [focusedIndex, providerSignature]);
    return const SizedBox.shrink();
  }
}

_TrailingImageGallery? _extractTrailingImageGallery(
  String content,
  Map<String, ImetaEntry> imetaByUrl,
) {
  final lines = content.split('\n');
  var cursor = lines.length - 1;
  while (cursor >= 0 && lines[cursor].trim().isEmpty) {
    cursor -= 1;
  }

  final items = <_MessageGalleryItem>[];
  final imagePattern = RegExp(r'^!\[([^\]]*)\]\((https?://[^)\s]+)\)$');
  while (cursor >= 0) {
    final match = imagePattern.firstMatch(lines[cursor].trim());
    if (match == null) break;
    final url = match.group(2)!;
    final imeta = imetaByUrl[url];
    if (classifyMediaUrl(url, imeta: imeta) == MessageMediaKind.video) {
      break;
    }
    final markdownLabel = match.group(1)?.trim();
    items.insert(
      0,
      _MessageGalleryItem(
        url: url,
        semanticLabel:
            imeta?.alt ??
            (markdownLabel?.isNotEmpty == true
                ? markdownLabel!
                : 'Message image'),
        aspectRatio: imeta?.aspectRatio,
      ),
    );
    cursor -= 1;
  }

  if (items.length < 2) return null;
  return _TrailingImageGallery(
    content: lines.take(cursor + 1).join('\n').trimRight(),
    items: items,
  );
}

class _MessageImageCarousel extends HookConsumerWidget {
  final List<_MessageGalleryItem> items;
  final double leadingOverflow;
  final double trailingOverflow;
  final VoidCallback? onReply;
  final MediaViewerMoreAction? onMore;

  const _MessageImageCarousel({
    super.key,
    required this.items,
    required this.leadingOverflow,
    required this.trailingOverflow,
    required this.onReply,
    required this.onMore,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final itemSignature = items.map((item) => item.url).join('\u0000');
    final heroTags = useMemoized(
      () => [for (var index = 0; index < items.length; index++) Object()],
      [itemSignature],
    );
    final controller = usePageController(viewportFraction: 0.9);
    final currentIndex = useState(0);
    final mediaAuth = ref.watch(mediaGetAuthServiceProvider);
    final mediaClient = ref.watch(mediaHttpClientProvider);

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            '${items.length} images',
            key: const ValueKey('message-media-carousel-count'),
            style: context.textTheme.labelMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
              fontWeight: FontWeight.w400,
            ),
          ),
          const SizedBox(height: Grid.half + Grid.quarter),
          LayoutBuilder(
            builder: (context, constraints) {
              final contentWidth = constraints.hasBoundedWidth
                  ? constraints.maxWidth
                  : _messageMediaMaxWidth(context);
              final carouselWidth =
                  contentWidth + leadingOverflow + trailingOverflow;
              final leadingExtent = leadingOverflow;
              final isLeftToRight =
                  Directionality.of(context) == TextDirection.ltr;
              final itemTrailingPaddings = [
                for (var index = 0; index < items.length; index++)
                  index == items.length - 1 ? Grid.gutter : Grid.half,
              ];
              final previewDecodeWidths = [
                for (var index = 0; index < items.length; index++)
                  math.max(
                    1.0,
                    carouselWidth * controller.viewportFraction -
                        itemTrailingPaddings[index],
                  ),
              ];
              final devicePixelRatio = MediaQuery.devicePixelRatioOf(context);
              final previewProviders = [
                for (var index = 0; index < items.length; index++)
                  ResizeImage.resizeIfNeeded(
                    (previewDecodeWidths[index] * devicePixelRatio).ceil(),
                    null,
                    MediaImageProvider(
                      url: items[index].url,
                      auth: mediaAuth,
                      client: mediaClient,
                    ),
                  ),
              ];
              final viewerItems = [
                for (var index = 0; index < items.length; index++)
                  MediaViewerImage(
                    url: items[index].url,
                    heroTag: heroTags[index],
                    semanticLabel: items[index].semanticLabel,
                    previewDecodeWidth: previewDecodeWidths[index],
                    aspectRatio: items[index].aspectRatio,
                    preloadProvider: previewProviders[index],
                  ),
              ];
              final carousel = SizedBox(
                key: const ValueKey('message-media-carousel'),
                width: carouselWidth,
                height: _messageMediaCarouselHeight,
                child: PageView.builder(
                  controller: controller,
                  allowImplicitScrolling: true,
                  // Keep the first image aligned with the message body, but
                  // allow later pages to paint through the avatar gutter as
                  // the row scrolls.
                  clipBehavior: Clip.none,
                  padEnds: false,
                  itemCount: items.length,
                  onPageChanged: (index) => currentIndex.value = index,
                  itemBuilder: (context, index) {
                    final item = items[index];
                    return Padding(
                      key: ValueKey('message-media-carousel-page:${item.url}'),
                      padding: EdgeInsetsDirectional.only(
                        end: itemTrailingPaddings[index],
                      ),
                      child: Semantics(
                        button: true,
                        excludeSemantics: true,
                        label: 'Open ${item.semanticLabel}',
                        child: GestureDetector(
                          key: ValueKey(
                            'message-media-carousel-item:${item.url}',
                          ),
                          onTap: () => openImageViewer(
                            context,
                            imageUrl: item.url,
                            heroTag: heroTags[index],
                            semanticLabel: item.semanticLabel,
                            previewDecodeWidth: previewDecodeWidths[index],
                            aspectRatio: item.aspectRatio,
                            galleryItems: viewerItems,
                            initialIndex: index,
                            onReply: onReply,
                            onMore: onMore,
                          ),
                          child: Container(
                            clipBehavior: Clip.antiAlias,
                            decoration: BoxDecoration(
                              color: context.colors.surfaceContainerHighest,
                              borderRadius: BorderRadius.circular(Radii.md),
                              border: Border.all(
                                color: context.colors.outlineVariant,
                              ),
                            ),
                            child: MediaViewerHero(
                              tag: heroTags[index],
                              child: ClipRRect(
                                borderRadius: BorderRadius.circular(Radii.md),
                                child: MediaImage(
                                  url: item.url,
                                  decodeWidth: previewDecodeWidths[index],
                                  fit: BoxFit.cover,
                                  semanticLabel: item.semanticLabel,
                                  errorBuilder: (_, _, _) =>
                                      const _MediaPreviewFallback(
                                        icon: LucideIcons.imageOff,
                                        label: 'Image unavailable',
                                      ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    );
                  },
                ),
              );

              final carouselSurface = Stack(
                clipBehavior: Clip.none,
                children: [
                  carousel,
                  _MessageGalleryPrecache(
                    providers: previewProviders,
                    focusedIndex: currentIndex.value,
                  ),
                ],
              );

              if (leadingExtent <= 0 && trailingOverflow <= 0) {
                return carouselSurface;
              }
              return SizedBox(
                width: contentWidth,
                height: _messageMediaCarouselHeight,
                child: OverflowBox(
                  alignment: AlignmentDirectional.centerStart,
                  minWidth: carouselWidth,
                  maxWidth: carouselWidth,
                  child: Transform.translate(
                    offset: Offset(
                      isLeftToRight ? -leadingExtent : leadingExtent,
                      0,
                    ),
                    child: carouselSurface,
                  ),
                ),
              );
            },
          ),
        ],
      ),
    );
  }
}
