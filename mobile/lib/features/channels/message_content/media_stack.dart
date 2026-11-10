part of '../message_content.dart';

/// A compact rounded photo fan with continuous perspective and depth overlap.
class _MessageImageStack extends HookConsumerWidget {
  const _MessageImageStack({
    super.key,
    required this.items,
    required this.mirrored,
    this.onReply,
    this.onMore,
    this.nativeActions,
    this.showTuningControl = true,
  });

  final List<MessageGalleryItem> items;
  final bool mirrored;
  final bool showTuningControl;
  final VoidCallback? onReply;
  final MediaViewerMoreAction? onMore;
  final NativePhotoActions? nativeActions;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final signature = items.map((item) => item.url).join('\u0000');
    final tags = useMemoized(() => [for (final _ in items) Object()], [
      signature,
    ]);
    final sourceKeys = useMemoized(() => [for (final _ in items) GlobalKey()], [
      signature,
    ]);
    final opening = useRef(false);
    final dragging = useRef(false);
    final position = useAnimationController(
      lowerBound: double.negativeInfinity,
      upperBound: double.infinity,
      keys: [signature],
    );
    final value = useAnimation(position);
    final tuning = useValueListenable(_stackTuning);
    final start = useRef(0.0);
    final distance = useRef(0.0);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final auth = ref.watch(mediaGetAuthServiceProvider);
    final index = value.round() % items.length;

    void settle(double target, {bool instant = false}) {
      if (reducedMotion || instant) {
        position.value = target;
      } else {
        unawaited(
          position.animateWith(
            ScrollSpringSimulation(
              const SpringDescription(mass: 1, stiffness: 300, damping: 32),
              position.value,
              target,
              0,
            ),
          ),
        );
      }
    }

    return Padding(
      padding: EdgeInsets.zero,
      child: LayoutBuilder(
        builder: (context, constraints) {
          // Reserve the full swept silhouette, including shadows, inside the row.
          final width = math.min(constraints.maxWidth, 340.0);
          final cardWidth = math.min(210.0, width * .62);
          final cardHeight = cardWidth;
          final height = cardHeight + Grid.xs * 2 + (mirrored ? 16 : 24);
          final decodeWidth = cardWidth;
          final physicalDecodeWidth =
              (cardWidth * MediaQuery.devicePixelRatioOf(context))
                  .ceilToDouble();
          final providers = [
            for (final item in items)
              ResizeImage.resizeIfNeeded(
                physicalDecodeWidth.toInt(),
                null,
                MediaImageProvider(
                  url: item.url,
                  auth: auth,
                  client: ref.watch(mediaHttpClientProvider),
                ),
              ),
          ];
          final viewerItems = [
            for (var i = 0; i < items.length; i++)
              MediaViewerImage(
                url: items[i].url,
                heroTag: tags[i],
                semanticLabel: items[i].semanticLabel,
                previewDecodeWidth: decodeWidth,
                aspectRatio: items[i].aspectRatio,
                preloadProvider: providers[i],
              ),
          ];
          Future<void> open() async {
            if (opening.value) return;
            opening.value = true;
            dragging.value = false;
            try {
              final selected = position.value.round() % items.length;
              settle(position.value.roundToDouble(), instant: true);
              if (NativePhotoZoomPreview.enabled) {
                await WidgetsBinding.instance.endOfFrame;
                if (!context.mounted) return;
                final gallery = await NativePhotoZoomPreview.open(
                  context: context,
                  sourceKeys: sourceKeys,
                  previewProvider: providers[selected],
                  previewProviders: providers,
                  providers: [
                    for (final item in items)
                      ResizeImage.resizeIfNeeded(
                        2048,
                        null,
                        MediaImageProvider(
                          url: item.url,
                          auth: auth,
                          client: ref.read(mediaHttpClientProvider),
                        ),
                      ),
                  ],
                  urls: [for (final item in items) item.url],
                  mimeTypes: [for (final item in items) item.mimeType],
                  labels: [for (final item in items) item.semanticLabel],
                  initialIndex: selected,
                  onSelect: (index) async {
                    if (!context.mounted) return;
                    dragging.value = false;
                    start.value = index.toDouble();
                    distance.value = 0;
                    settle(index.toDouble(), instant: true);
                    await WidgetsBinding.instance.endOfFrame;
                  },
                  onReply: onReply,
                  actions: nativeActions,
                );
                if (!gallery || !context.mounted) return;
              }
              openImageViewer(
                context,
                imageUrl: items[selected].url,
                heroTag: tags[selected],
                semanticLabel: items[selected].semanticLabel,
                previewDecodeWidth: decodeWidth,
                aspectRatio: items[selected].aspectRatio,
                galleryItems: viewerItems,
                initialIndex: selected,
                onSelectionChanged: (index) {
                  if (!context.mounted) return;
                  dragging.value = false;
                  start.value = index.toDouble();
                  distance.value = 0;
                  settle(index.toDouble(), instant: true);
                },
                onReply: onReply,
                onMore: onMore,
              );
            } finally {
              opening.value = false;
            }
          }

          // Keep the next hidden card underneath the third slot throughout
          // the handoff. It emerges as that slot advances, without a mid-swipe swap.

          final offsets = <int, double>{};
          for (var n = 0; n <= 3; n++) {
            final absolute = value.floor() + n;
            final i = absolute % items.length;
            final delta = absolute - value;
            if (!offsets.containsKey(i) || delta.abs() < offsets[i]!.abs()) {
              offsets[i] = delta;
            }
          }
          final departing = value.floor() % items.length;
          final arriving = (value.floor() + 1) % items.length;
          final fraction = value - value.floor();
          offsets[departing] = -fraction;
          if (fraction > 0) offsets[arriving] = 1 - fraction;
          // Use integer slots, not animated distances: equal-depth cards must
          // not trade layers as floating-point values cross during a drag.
          final order = offsets.keys.toList()
            ..sort((a, b) {
              final aSlot = (offsets[a]! + fraction).round();
              final bSlot = (offsets[b]! + fraction).round();
              final depth = bSlot.abs().compareTo(aSlot.abs());
              return depth != 0 ? depth : bSlot.compareTo(aSlot);
            });
          final radius = BorderRadius.circular(Radii.container + Grid.xxs);
          final poses = {
            for (final i in order)
              i: _StackCardPose(
                offsets[i]!,
                items.length,
                cardWidth,
                mirrored,
                fraction,
                adjustments: _stackTuningEnabled ? tuning : null,
              ),
          };
          // Keep the front pair ordered even arbitrarily close to an integer.
          // A transition epsilon briefly put the old photo back on top during
          // the spring's last frames, flashing it before the new index settled.
          order.remove(departing);
          order.remove(arriving);
          if (fraction >= .5) {
            order.insert(0, departing);
            order.add(arriving);
          } else {
            order.addAll(
              fraction < .5 ? [arriving, departing] : [departing, arriving],
            );
          }
          Widget card(int i) {
            final photo = MediaImage(
              url: items[i].url,
              decodeWidth: decodeWidth,
              fit: BoxFit.cover,
              semanticLabel: items[i].semanticLabel,
              frameBuilder: (context, child, frame, synchronous) =>
                  frame != null || synchronous
                  ? child
                  : const MediaLoadingPlaceholder(label: 'Loading image'),
              errorBuilder: (_, _, _) => const _MediaPreviewFallback(
                icon: BuzzIcons.imageOff,
                label: 'Image unavailable',
              ),
            );
            return _StackPhotoCard(
              key: ValueKey('stack-card:$i'),
              pose: poses[i]!,
              mirrored: mirrored,
              width: cardWidth,
              height: cardHeight,
              radius: radius,
              shadow: items.length <= 3
                  ? 1.0
                  : offsets[i]! < 0
                  ? (1 + offsets[i]!).clamp(0.0, 1.0)
                  : (3 - offsets[i]!).clamp(0.0, 1.0),
              child: SizedBox(
                key: sourceKeys[i],
                child: MediaViewerHero(tag: tags[i], child: photo),
              ),
            );
          }

          return Align(
            alignment: mirrored ? Alignment.centerRight : Alignment.centerLeft,
            child: Semantics(
              key: const ValueKey('message-media-stack-semantics'),
              button: true,
              label: items[index].semanticLabel,
              hint: 'Open photo gallery',
              customSemanticsActions: _stackTuningEnabled && showTuningControl
                  ? {
                      CustomSemanticsAction(label: 'Tune stack'): () =>
                          _openStackTuning(context, items, mirrored),
                    }
                  : null,
              value: 'Photo ${index + 1} of ${items.length}',
              increasedValue:
                  'Photo ${(index + 1) % items.length + 1} of ${items.length}',
              decreasedValue:
                  'Photo ${(index - 1) % items.length + 1} of ${items.length}',
              onTap: open,
              onIncrease: () => settle(position.value.roundToDouble() + 1),
              onDecrease: () => settle(position.value.roundToDouble() - 1),
              excludeSemantics: true,
              child: Focus(
                onKeyEvent: (_, event) {
                  if (event is! KeyDownEvent) {
                    return KeyEventResult.ignored;
                  }
                  if (event.logicalKey == LogicalKeyboardKey.arrowRight ||
                      event.logicalKey == LogicalKeyboardKey.arrowLeft) {
                    settle(
                      position.value.roundToDouble() +
                          (event.logicalKey == LogicalKeyboardKey.arrowRight
                              ? 1
                              : -1),
                      instant: true,
                    );
                    return KeyEventResult.handled;
                  }
                  if (event.logicalKey == LogicalKeyboardKey.enter ||
                      event.logicalKey == LogicalKeyboardKey.space) {
                    open();
                    return KeyEventResult.handled;
                  }
                  return KeyEventResult.ignored;
                },
                child: GestureDetector(
                  key: const ValueKey('message-media-stack'),
                  behavior: HitTestBehavior.opaque,
                  onTap: open,
                  onHorizontalDragStart: (_) {
                    if (opening.value) return;
                    dragging.value = true;
                    position.stop();
                    start.value = position.value;
                    distance.value = 0;
                  },
                  onHorizontalDragUpdate: (details) {
                    if (!dragging.value || opening.value) return;
                    distance.value += details.delta.dx;
                    if (!reducedMotion) {
                      position.value = start.value - distance.value / 140;
                    }
                  },
                  onHorizontalDragEnd: (details) {
                    if (!dragging.value || opening.value) return;
                    dragging.value = false;
                    final dx = distance.value;
                    final speed = details.primaryVelocity ?? 0;
                    final step = dx.abs() > 48 || speed.abs() > 500
                        ? (dx == 0 ? -speed.sign : -dx.sign) *
                              math.max(1, (dx.abs() / 140).round())
                        : 0;
                    settle(start.value.roundToDouble() + step);
                  },
                  onHorizontalDragCancel: () {
                    if (!dragging.value || opening.value) return;
                    dragging.value = false;
                    settle(start.value.roundToDouble());
                  },
                  child: SizedBox(
                    width: width,
                    height: height,
                    child: Stack(
                      key: const ValueKey('message-media-card-deck'),
                      clipBehavior: Clip.none,
                      alignment: mirrored
                          ? Alignment.centerRight
                          : Alignment.centerLeft,
                      children: [
                        for (final i in order) card(i),
                        if (_stackTuningEnabled && showTuningControl)
                          Positioned(
                            left: 0,
                            top: 0,
                            child: TextButton(
                              onPressed: () =>
                                  _openStackTuning(context, items, mirrored),
                              child: const Text('Tune stack'),
                            ),
                          ),
                        _MessageGalleryPrecache(
                          providers: providers,
                          focusedIndex: index,
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// Keep both photos solid. Their projected edges meet at the layer exchange,
/// then the arriving card's rounded edge moves across the departing photo.
class _StackCardPose {
  _StackCardPose(
    double delta,
    int count,
    double width,
    bool mirrored,
    double phase, {
    List<_StackAdjustment>? adjustments,
  }) {
    var transform = _transform(
      delta,
      count,
      width,
      mirrored,
      phase: phase,
      adjustments: adjustments,
    );
    if (delta < 0 && delta > -1) {
      final bounds = Rect.fromCenter(
        center: Offset.zero,
        width: width,
        height: width,
      );
      final outgoing = MatrixUtils.transformRect(
        _transform(-.5, count, width, mirrored, adjustments: adjustments),
        bounds,
      );
      final incoming = MatrixUtils.transformRect(
        _transform(.5, count, width, mirrored, adjustments: adjustments),
        bounds,
      );
      final overlap = mirrored
          ? outgoing.right - incoming.left
          : incoming.right - outgoing.left;
      // Zero velocity at the exchange keeps the seam from jumping. Only the
      // outgoing card receives the extra travel; the next card advances in place.
      final arc = math.pow(math.sin(-delta * math.pi), 2).toDouble();
      transform = Matrix4.translationValues(
        (mirrored ? -1 : 1) * math.max(0, overlap) * arc,
        0,
        0,
      )..multiply(transform);
    }
    if (delta < -.75 && delta > -1) {
      // Finish underneath the persistent rear card before recycling this image.
      // Matching its entire pose prevents an exposed edge from disappearing.
      final rear = _transform(
        math.min(count, 3) + delta,
        count,
        width,
        mirrored,
        phase: phase,
        adjustments: adjustments,
      );
      final t = Curves.easeInOut.transform(
        ((-delta - .75) / .2).clamp(0.0, 1.0),
      );
      for (var i = 0; i < 16; i++) {
        transform.storage[i] += (rear.storage[i] - transform.storage[i]) * t;
      }
    }
    matrix = transform;
  }

  static Matrix4 _transform(
    double delta,
    int count,
    double width,
    bool mirrored, {
    double phase = .5,
    List<_StackAdjustment>? adjustments,
  }) {
    final depth = (delta < 0 ? -delta * (math.min(count, 3) - 1) : delta.abs())
        .clamp(0.0, 2.0);
    final adjustment = _StackAdjustment.at(
      adjustments ??
          (mirrored ? _sentStackAdjustments : _receivedStackAdjustments),
      depth,
    );
    final active = delta.abs() < 1;
    // Rear cards travel with the incoming deck, rather than remaining flat
    // while only the two leading photos rotate. The shared phase also keeps
    // their movement continuous when a card crosses a rear depth slot.
    final excursion = math.sin((active ? delta.abs() : phase) * math.pi);
    final departing = active && delta < 0;
    final direction = mirrored ? 1.0 : -1.0;
    // Two backing slots lean in opposite directions. Deeper photos share
    // the third card's exact pose, fully covered until that card advances.
    final fan = depth <= 1 ? .08 * depth : .08 - .16 * (depth - 1);
    final backOffset = depth <= 1 ? -8 * depth : -8 + 16 * (depth - 1);
    return Matrix4.identity()
      ..setEntry(3, 2, .0008)
      ..translateByDouble(
        direction *
                (width * (departing ? -.36 : .11) * excursion + backOffset) +
            adjustment.x,
        depth * 2 + adjustment.y,
        0,
        1,
      )
      ..rotateY(direction * (departing ? -.95 : .75) * excursion)
      ..rotateZ(direction * fan + adjustment.degrees * math.pi / 180)
      ..scaleByDouble(1 - .025 * depth, 1 - .025 * depth, 1, 1);
  }

  late final Matrix4 matrix;
}

class _StackPhotoCard extends StatelessWidget {
  const _StackPhotoCard({
    super.key,
    required this.pose,
    required this.mirrored,
    required this.width,
    required this.height,
    required this.radius,
    required this.shadow,
    required this.child,
  });
  final _StackCardPose pose;
  final bool mirrored;
  final double shadow;
  final double width, height;
  final BorderRadius radius;
  final Widget child;

  @override
  Widget build(BuildContext context) => Transform.translate(
    offset: Offset(
      mirrored ? -(Grid.xs + Grid.twelve) : Grid.xs + Grid.twelve,
      -Grid.half,
    ),
    child: Transform(
      alignment: Alignment.center,
      transform: pose.matrix,
      child: SizedBox(
        width: width,
        height: height,
        child: DecoratedBox(
          decoration: ShapeDecoration(
            color: context.colors.surface,
            shape: RoundedSuperellipseBorder(borderRadius: radius),
            shadows: shadow > 0
                ? [
                    BoxShadow(
                      color: const Color(
                        0x30000000,
                      ).withValues(alpha: 48 / 255 * shadow),
                      blurRadius: 12,
                      offset: Offset(0, 6),
                    ),
                  ]
                : null,
          ),
          child: RepaintBoundary(
            child: Padding(
              padding: const EdgeInsets.all(Grid.half),
              child: ClipRSuperellipse(
                borderRadius: radius - BorderRadius.circular(Grid.half),
                child: child,
              ),
            ),
          ),
        ),
      ),
    ),
  );
}
