part of '../message_content.dart';

class _MessageGalleryPrecache extends HookWidget {
  final List<ImageProvider<Object>> providers;
  final int focusedIndex;

  const _MessageGalleryPrecache({
    required this.providers,
    required this.focusedIndex,
  });

  @override
  Widget build(BuildContext context) {
    final prefetch = useMemoized(_GalleryPrecache.new, const []);
    useEffect(() => prefetch.dispose, [prefetch]);
    final providerSignature = Object.hashAll(providers);
    useEffect(() {
      var cancelled = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (cancelled || !context.mounted) return;
        // Prepare a five-photo strip immediately, then keep four ahead and two
        // behind the current page. Two workers bound speculative network work.
        final indices = [
          for (var i = focusedIndex; i <= focusedIndex + 4; i++) i,
          focusedIndex - 1,
          focusedIndex - 2,
        ];
        prefetch.update(context, [
          for (final i in indices)
            if (i >= 0 && i < providers.length) providers[i],
        ]);
      });
      return () => cancelled = true;
    }, [focusedIndex, providerSignature]);
    return const SizedBox.shrink();
  }
}

class _GalleryPrecache {
  BuildContext? _context;
  List<ImageProvider<Object>> _window = [];
  final _ready = <ImageProvider<Object>>{};
  final _loading = <ImageProvider<Object>>{};

  void update(BuildContext context, List<ImageProvider<Object>> window) {
    _context = context;
    _window = window;
    _ready.removeWhere((provider) => !window.contains(provider));
    _drain();
  }

  void _drain() {
    final context = _context;
    if (context == null || !context.mounted) return;
    for (final provider in _window) {
      if (_loading.length >= 2) break;
      if (_ready.contains(provider) || _loading.contains(provider)) continue;
      _loading.add(provider);
      unawaited(_load(context, provider));
    }
  }

  Future<void> _load(
    BuildContext context,
    ImageProvider<Object> provider,
  ) async {
    await precacheImage(provider, context, onError: (_, _) {});
    _loading.remove(provider);
    if (_window.contains(provider)) _ready.add(provider);
    _drain();
  }

  void dispose() {
    _context = null;
    _window = [];
    _ready.clear();
  }
}

class _MessageImageCarousel extends HookConsumerWidget {
  final List<MessageGalleryItem> items;
  final bool mirrored;
  final double leadingOverflow;
  final double trailingOverflow;
  final VoidCallback? onReply;
  final MediaViewerMoreAction? onMore;

  const _MessageImageCarousel({
    super.key,
    required this.items,
    required this.mirrored,
    required this.leadingOverflow,
    required this.trailingOverflow,
    required this.onReply,
    required this.onMore,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final viewerContext = context;
    final itemSignature = items.map((item) => item.url).join('\u0000');
    final heroTags = useMemoized(
      () => [for (var index = 0; index < items.length; index++) Object()],
      [itemSignature],
    );
    final currentIndex = useState(0);
    final mediaAuth = ref.watch(mediaGetAuthServiceProvider);
    final mediaClient = ref.watch(mediaHttpClientProvider);

    return MessageGalleryFrame(
      count: items.length,
      mirrored: mirrored,
      child: Directionality(
        textDirection: mirrored ? TextDirection.rtl : TextDirection.ltr,
        child: LayoutBuilder(
          builder: (context, constraints) {
            final contentWidth = constraints.hasBoundedWidth
                ? constraints.maxWidth
                : messageMediaMaxWidth(context);
            final carouselWidth =
                contentWidth + leadingOverflow + trailingOverflow;
            // Directionality mirrors both the starting edge and trailing overflow.
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
                  carouselWidth * 0.9 - itemTrailingPaddings[index],
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
            final nativeGutter = math.max(
              0.0,
              MediaQuery.sizeOf(context).width - carouselWidth,
            );
            final carousel = SizedBox(
              key: const ValueKey('message-media-carousel'),
              width: carouselWidth,
              height: messageMediaCarouselHeight,
              child: !kIsWeb && defaultTargetPlatform == TargetPlatform.iOS
                  ? OverflowBox(
                      minWidth: carouselWidth + 2 * nativeGutter,
                      maxWidth: carouselWidth + 2 * nativeGutter,
                      child: IosMediaCarousel(
                        key: ValueKey((itemSignature, mediaAuth)),
                        providers: previewProviders,
                        labels: [for (final item in items) item.semanticLabel],
                        width: carouselWidth,
                        gutter: nativeGutter,
                        gap: Grid.half,
                        trailingPadding: Grid.gutter,
                        radius:
                            (MessageAttachmentCorners.of(context) ??
                                    BorderRadius.circular(Radii.md))
                                .topLeft
                                .x,
                        onScroll: (offset) => MessageGalleryScrollNotification(
                          offset,
                        ).dispatch(context),
                        onPageChanged: (index) {
                          if (index != currentIndex.value) {
                            unawaited(HapticFeedback.selectionClick());
                          }
                          currentIndex.value = index;
                        },
                        onOpen: (index) => openImageViewer(
                          viewerContext,
                          imageUrl: items[index].url,
                          heroTag: heroTags[index],
                          semanticLabel: items[index].semanticLabel,
                          previewDecodeWidth: previewDecodeWidths[index],
                          aspectRatio: items[index].aspectRatio,
                          galleryItems: viewerItems,
                          initialIndex: index,
                          onReply: onReply,
                          onMore: onMore,
                        ),
                      ),
                    )
                  : _MessageCarouselViewport(
                      width: carouselWidth,
                      currentIndex: currentIndex.value,
                      itemCount: items.length,
                      onPageChanged: (index) {
                        if (index != currentIndex.value) {
                          unawaited(HapticFeedback.selectionClick());
                        }
                        currentIndex.value = index;
                      },
                      itemBuilder: (context, index) {
                        final item = items[index];
                        return Padding(
                          key: ValueKey(
                            'message-media-carousel-page:${item.url}',
                          ),
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
                                viewerContext,
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
                                  borderRadius:
                                      MessageAttachmentCorners.of(context) ??
                                      BorderRadius.circular(Radii.md),
                                  border: Border.all(
                                    color: context.colors.outlineVariant,
                                  ),
                                ),
                                child: MediaViewerHero(
                                  tag: heroTags[index],
                                  child: ClipRRect(
                                    borderRadius:
                                        MessageAttachmentCorners.of(context) ??
                                        BorderRadius.circular(Radii.md),
                                    child: MediaImage(
                                      url: item.url,
                                      decodeWidth: previewDecodeWidths[index],
                                      fit: BoxFit.cover,
                                      semanticLabel: item.semanticLabel,
                                      frameBuilder:
                                          (
                                            context,
                                            child,
                                            frame,
                                            synchronous,
                                          ) => frame != null || synchronous
                                          ? child
                                          : const MediaLoadingPlaceholder(
                                              label: 'Loading image',
                                            ),
                                      errorBuilder: (_, _, _) =>
                                          const _MediaPreviewFallback(
                                            icon: BuzzIcons.imageOff,
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
              height: messageMediaCarouselHeight,
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
      ),
    );
  }
}

/// Extends the actual paint viewport through the message gutters. Clip.none
/// alone cannot keep a sliver child painting once it leaves its viewport.
class _MessageCarouselViewport extends HookWidget {
  const _MessageCarouselViewport({
    required this.width,
    required this.currentIndex,
    required this.itemCount,
    required this.onPageChanged,
    required this.itemBuilder,
  });

  final double width;
  final int currentIndex;
  final int itemCount;
  final ValueChanged<int> onPageChanged;
  final IndexedWidgetBuilder itemBuilder;

  @override
  Widget build(BuildContext context) {
    // Cover any space between the message body and either screen edge.
    // Matching sliver padding preserves the original alignment, page stride,
    // and end-of-gallery scroll limit despite the wider paint viewport.
    final gutter = math.max(0.0, MediaQuery.sizeOf(context).width - width);
    final viewportWidth = width + 2 * gutter;
    final fraction = width * 0.9 / viewportWidth;
    final controller = usePageController(
      initialPage: currentIndex,
      viewportFraction: fraction,
      keys: [fraction],
    );

    return OverflowBox(
      minWidth: viewportWidth,
      maxWidth: viewportWidth,
      child: NotificationListener<ScrollUpdateNotification>(
        onNotification: (notification) {
          if (notification.depth == 0 && notification.metrics is PageMetrics) {
            MessageGalleryScrollNotification(
              notification.metrics.pixels -
                  notification.metrics.minScrollExtent,
            ).dispatch(context);
            final index = (notification.metrics as PageMetrics).page!.round();
            if (index != currentIndex) onPageChanged(index);
          }
          return false;
        },
        child: CustomScrollView(
          controller: controller,
          scrollDirection: Axis.horizontal,
          physics: _CarouselPagePhysics(
            parent: Theme.of(context).platform == TargetPlatform.iOS
                ? const BouncingScrollPhysics()
                : const ClampingScrollPhysics(),
          ),
          clipBehavior: Clip.none,
          slivers: [
            SliverPadding(
              padding: EdgeInsets.symmetric(horizontal: gutter),
              sliver: SliverFillViewport(
                viewportFraction: fraction,
                padEnds: false,
                delegate: SliverChildBuilderDelegate(
                  itemBuilder,
                  childCount: itemCount,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _CarouselPagePhysics extends PageScrollPhysics {
  const _CarouselPagePhysics({super.parent});

  @override
  bool get allowImplicitScrolling => true;

  @override
  _CarouselPagePhysics applyTo(ScrollPhysics? ancestor) =>
      _CarouselPagePhysics(parent: buildParent(ancestor));
}
