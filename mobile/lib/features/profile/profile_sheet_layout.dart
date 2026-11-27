import 'package:flutter/gestures.dart' show Drag;
import 'dart:math' as math;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'profile_artwork_theme.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/widgets/agent_artwork_panel.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/buzz_sheet_header.dart';
import '../../shared/widgets/buzz_navigation_metrics.dart';
import '../../shared/widgets/buzz_toast.dart';
import '../../shared/widgets/native_profile_sheet.dart';
import '../../shared/widgets/navigation_backdrop.dart';
import '../../shared/widgets/ios_navigation_bar.dart';

const _compactHeight = 340.0;
const _navigationHeight = 60.0;

class ProfileSheetLayout extends HookConsumerWidget {
  const ProfileSheetLayout({
    super.key,
    required this.hero,
    required this.body,
    this.avatarUrl,
    this.photoOpen = false,
    this.enlargedPhoto,
    this.portraitKey,
    this.onPhotoBack,
    this.onPhotoClosed,
    this.status,
    this.memberActions,
    this.leading,
    this.embedded = false,
    this.sheetControls = true,
    this.initiallyExpanded = false,
    this.agent = false,
    this.contentPadding = EdgeInsets.zero,
  });
  final Widget hero, body;
  final bool photoOpen;
  final Widget? enlargedPhoto, status, memberActions, leading;
  final GlobalKey? portraitKey;
  final VoidCallback? onPhotoBack, onPhotoClosed;
  final String? avatarUrl;
  final bool embedded, sheetControls, initiallyExpanded, agent;
  final EdgeInsets contentPadding;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final native = NativeProfileSheet.maybeOf(context);
    final showGrabber = !useValueListenable(buzzToastVisibleOf(context));
    final lastDimensions = useRef<(double, double, bool, bool, Rect?)?>(null);
    final contentTopInset =
        (sheetControls ? _navigationHeight : 0.0) +
        (!agent && Theme.of(context).platform == TargetPlatform.iOS
            ? 4.0
            : 0.0);
    final documentKey = useMemoized(GlobalKey.new);
    final headerDrag = useRef<Drag?>(null);
    useEffect(
      () =>
          () => headerDrag.value?.cancel(),
      const [],
    );
    final nativeScroll = useMemoized(_NativeScrollController.new);
    useEffect(() => nativeScroll.dispose, [nativeScroll]);
    final active = (ModalRoute.isCurrentOf(context) ?? true) && !photoOpen;
    useEffect(() {
      void scroll() {
        if (native == null || !native.focused || !nativeScroll.hasClients) {
          return;
        }
        (nativeScroll.position as _NativeScrollPosition).follow(
          native.offset.value,
        );
      }

      native?.offset.addListener(scroll);
      return () => native?.offset.removeListener(scroll);
    }, [native?.offset, native?.focused, nativeScroll]);
    final statusKey = useMemoized(GlobalKey.new);
    final actionsKey = useMemoized(GlobalKey.new);
    final compactExtraHeight = useState(0.0);
    final largerPortrait =
        !agent && Theme.of(context).platform == TargetPlatform.iOS;
    final baseHeight =
        _compactHeight +
        (largerPortrait ? teamPortraitSize - profilePortraitSize + 8 : 0);
    final compactHeight = baseHeight + compactExtraHeight.value;
    final heroKey = useMemoized(GlobalKey.new);
    final heroHeight = useState(280.0);
    final detents = useMemoized(DraggableScrollableController.new);
    useEffect(() => detents.dispose, [detents]);
    final useDetents = sheetControls && !embedded;
    void measureHeader() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        final box = heroKey.currentContext?.findRenderObject();
        final bubble = statusKey.currentContext?.findRenderObject();
        final actions = actionsKey.currentContext?.findRenderObject();
        final bubbleHeight = bubble is RenderBox && bubble.hasSize
            ? bubble.size.height
            : 0.0;
        final extra =
            bubbleHeight +
            (actions is RenderBox && actions.hasSize
                ? actions.size.height
                : 0.0);
        if (extra != compactExtraHeight.value) compactExtraHeight.value = extra;
        final document = documentKey.currentContext?.findRenderObject();
        if (native != null &&
            native.focused &&
            box is RenderBox &&
            document is RenderBox &&
            box.hasSize &&
            document.hasSize) {
          final strip = native.rosterKey?.currentContext?.findRenderObject();
          final roster = active && strip is RenderBox && strip.hasSize
              ? (strip.localToGlobal(Offset.zero) +
                        Offset(
                          0,
                          nativeScroll.hasClients ? nativeScroll.offset : 0,
                        )) &
                    strip.size
              : null;
          final dimensions = (
            baseHeight + extra,
            contentTopInset + document.size.height,
            active,
            showGrabber,
            roster,
          );
          if (lastDimensions.value != dimensions) {
            lastDimensions.value = dimensions;
            native.setDimensions(
              dimensions.$1,
              dimensions.$2,
              dimensions.$3,
              showGrabber: showGrabber,
              roster: roster,
            );
          }
        }
        if (box is RenderBox &&
            box.hasSize &&
            box.size.height != heroHeight.value) {
          heroHeight.value = box.size.height;
        }
      });
    }

    measureHeader();
    final sectionTheme = useProfileArtworkTheme(
      context,
      ref,
      avatarUrl,
      personProfile: !agent,
    );
    final panelKey = useMemoized(GlobalKey.new);
    final photoOrigin = useRef(Rect.zero);
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 240);
    useEffect(() {
      if (!photoOpen || !useDetents) return null;
      if (native != null) {
        native.setPhotoOpen(true);
        return () {
          native.setPhotoOpen(false);
        };
      }
      if (detents.isAttached) {
        if (duration == Duration.zero) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (context.mounted && detents.isAttached) detents.jumpTo(1);
          });
        } else {
          detents.animateTo(1, duration: duration, curve: Curves.easeOutCubic);
        }
      }
      return null;
    }, [photoOpen, native?.id, useDetents]);
    if (photoOpen) {
      final portrait = portraitKey?.currentContext?.findRenderObject();
      final panelBox = panelKey.currentContext?.findRenderObject();
      if (portrait is RenderBox && panelBox is RenderBox && portrait.hasSize) {
        photoOrigin.value =
            portrait.localToGlobal(Offset.zero, ancestor: panelBox) &
            portrait.size;
      }
    }
    Widget fade(Widget child) => IgnorePointer(
      ignoring: photoOpen,
      child: ExcludeSemantics(
        excluding: photoOpen,
        child: AnimatedOpacity(
          opacity: photoOpen ? 0 : 1,
          duration: duration,
          child: child,
        ),
      ),
    );
    final backdropHeight = contentTopInset + heroHeight.value;
    final backdrop = IgnorePointer(
      child: AgentArtworkPanel(
        key: const ValueKey('profile-artwork-backdrop'),
        imageUrl: avatarUrl,
        personProfile: !agent,
        borderRadius: BorderRadius.zero,
        child: const SizedBox.expand(),
      ),
    );
    final embeddedScroll = useScrollController();
    final movesWithPortrait =
        !agent && sectionTheme.platform == TargetPlatform.iOS;
    Widget scrollingBackdrop(ScrollController controller) => AnimatedBuilder(
      animation: controller,
      child: backdrop,
      builder: (context, child) {
        final offset = controller.hasClients ? controller.offset : 0.0;
        final fraction = backdropHeight > 0 ? offset / backdropHeight : 0.0;
        // Keep the artwork mounted as rubber-banding crosses the resting offset.
        return Transform.translate(
          offset: Offset(0, -offset.clamp(0.0, double.infinity)),
          child: Transform.scale(
            alignment: Alignment.topCenter,
            scale: fraction < 0 ? 1 - fraction : 1,
            child: Opacity(opacity: 1 - fraction.clamp(0.0, 1.0), child: child),
          ),
        );
      },
    );
    Widget panel(ScrollController? scroll, Widget header) => PopScope(
      canPop: !photoOpen,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && photoOpen) onPhotoBack?.call();
      },
      child: Theme(
        data: sectionTheme,
        child: Material(
          color: sectionTheme.scaffoldBackgroundColor,
          child: Stack(
            key: panelKey,
            children: [
              Positioned(
                top: 0,
                left: 0,
                right: 0,
                height: backdropHeight,
                child: fade(
                  movesWithPortrait
                      ? scrollingBackdrop(scroll ?? embeddedScroll)
                      : backdrop,
                ),
              ),
              fade(
                Padding(
                  padding: contentPadding,
                  child: SingleChildScrollView(
                    controller:
                        scroll ?? (movesWithPortrait ? embeddedScroll : null),
                    padding: EdgeInsets.only(top: contentTopInset),
                    physics: native != null
                        ? const NeverScrollableScrollPhysics()
                        : useDetents
                        ? const AlwaysScrollableScrollPhysics()
                        : null,
                    child: NotificationListener<SizeChangedLayoutNotification>(
                      onNotification: (_) {
                        measureHeader();
                        return false;
                      },
                      child: SizeChangedLayoutNotifier(
                        child: Column(
                          key: documentKey,
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            NotificationListener<SizeChangedLayoutNotification>(
                              onNotification: (_) {
                                measureHeader();
                                return false;
                              },
                              child: SizeChangedLayoutNotifier(
                                child: ConstrainedBox(
                                  key: heroKey,
                                  constraints: BoxConstraints(
                                    minHeight: useDetents
                                        ? compactHeight - _navigationHeight
                                        : 0,
                                  ),
                                  child: Align(
                                    heightFactor: 1,
                                    child: Padding(
                                      padding: EdgeInsets.fromLTRB(
                                        16,
                                        (sheetControls ? 8.0 : 44.0) +
                                            (agent ? 0 : 4),
                                        16,
                                        32,
                                      ),
                                      child: Column(
                                        mainAxisSize: MainAxisSize.min,
                                        children: [
                                          if (status case final bubble?)
                                            SizeChangedLayoutNotifier(
                                              key: statusKey,
                                              child: bubble,
                                            ),
                                          hero,
                                          if (memberActions case final actions?)
                                            SizeChangedLayoutNotifier(
                                              key: actionsKey,
                                              child: actions,
                                            ),
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                            if (sectionTheme.platform == TargetPlatform.iOS)
                              FlatListSections(child: body)
                            else
                              body,
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
              if (sheetControls)
                Positioned(
                  top: 0,
                  left: 0,
                  right: 0,
                  child: fade(
                    NavigationBackdrop(prefersSoftTopEdge: true, child: header),
                  ),
                ),
              if (enlargedPhoto != null)
                Positioned.fill(
                  child: TweenAnimationBuilder<double>(
                    tween: Tween<double>(begin: 0, end: photoOpen ? 1 : 0),
                    duration: duration,
                    curve: Curves.easeInOutCubic,
                    onEnd: photoOpen ? null : onPhotoClosed,
                    child: enlargedPhoto,
                    builder: (context, progress, photo) {
                      if (progress == 0) return const SizedBox.shrink();
                      return LayoutBuilder(
                        builder: (context, box) {
                          final visibleHeight = box.maxHeight;
                          final width =
                              box.maxWidth - contentPadding.horizontal;
                          final side = math.max(
                            0.0,
                            math.min(
                              360.0,
                              math.min(width - 32, visibleHeight - 88),
                            ),
                          );
                          final target = Rect.fromCenter(
                            center: Offset(
                              contentPadding.left + width / 2,
                              (visibleHeight + 44) / 2,
                            ),
                            width: side,
                            height: side,
                          );
                          final rect = Rect.lerp(
                            photoOrigin.value,
                            target,
                            progress,
                          )!;
                          final back =
                              Theme.of(context).platform == TargetPlatform.iOS
                              ? IosGlassNavigationButton(
                                  icon: IosGlassNavigationIcon.back,
                                  semanticLabel: 'Back to profile',
                                  width: buzzNavigationActionSize,
                                  height: buzzNavigationActionSize,
                                  controlSize: buzzNavigationActionSize,
                                  onPressed: onPhotoBack,
                                )
                              : BackButton(onPressed: onPhotoBack);
                          return Stack(
                            children: [
                              const Positioned.fill(
                                child: ModalBarrier(
                                  dismissible: false,
                                  color: Colors.transparent,
                                ),
                              ),
                              Positioned.fromRect(
                                rect: rect,
                                child: FittedBox(
                                  child: SizedBox.square(
                                    dimension: side,
                                    child: photo,
                                  ),
                                ),
                              ),
                              if (sheetControls)
                                Positioned(
                                  top: 0,
                                  left: 0,
                                  right: 0,
                                  child: Opacity(
                                    opacity: progress,
                                    child: BuzzSheetHeader(
                                      leading: SizedBox.square(
                                        dimension: buzzNavigationActionSize,
                                        child: back,
                                      ),
                                      trailing: const SizedBox.shrink(),
                                    ),
                                  ),
                                )
                              else
                                PositionedDirectional(
                                  start: contentPadding.left + 12,
                                  top: contentPadding.top + 8,
                                  child: Opacity(
                                    opacity: progress,
                                    child: back,
                                  ),
                                ),
                            ],
                          );
                        },
                      );
                    },
                  ),
                ),
            ],
          ),
        ),
      ),
    );
    if (native != null && useDetents) {
      return IosNavigationScrollScope(
        offset: native.offset,
        child: panel(
          nativeScroll,
          BuzzSheetHeader(
            leading: leading,
            closeControlSize: buzzNavigationActionSize,
          ),
        ),
      );
    }
    if (!useDetents) {
      return ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: embedded
              ? double.infinity
              : MediaQuery.sizeOf(context).height * .88,
        ),
        child: panel(
          null,
          BuzzSheetHeader(
            leading: leading,
            closeControlSize: buzzNavigationActionSize,
          ),
        ),
      );
    }
    return LayoutBuilder(
      builder: (context, box) {
        final height = box.maxHeight;
        final compact = (compactHeight / height).clamp(.12, 1.0);
        return DraggableScrollableSheet(
          controller: detents,
          initialChildSize: initiallyExpanded ? 1 : compact,
          minChildSize: compact,
          maxChildSize: 1,
          expand: false,
          snap: true,
          shouldCloseOnMinExtent: false,
          builder: (context, scroll) => panel(
            scroll,
            GestureDetector(
              behavior: HitTestBehavior.opaque,
              onVerticalDragStart: (details) {
                headerDrag.value = scroll.position.drag(
                  details,
                  () => headerDrag.value = null,
                );
              },
              onVerticalDragUpdate: (details) =>
                  headerDrag.value?.update(details),
              onVerticalDragCancel: () => headerDrag.value?.cancel(),
              onVerticalDragEnd: (details) {
                final dismiss =
                    detents.isAttached &&
                    detents.size <= compact + .001 &&
                    scroll.offset <= 0 &&
                    details.velocity.pixelsPerSecond.dy > 500;
                headerDrag.value?.end(details);
                if (dismiss) Navigator.of(context).maybePop();
              },
              child: BuzzSheetHeader(
                leading: leading,
                showDragHandle: showGrabber,
                closeControlSize: buzzNavigationActionSize,
              ),
            ),
          ),
        );
      },
    );
  }
}

// UIKit supplies every offset, including deceleration and rubber-banding.
class _NativeScrollController extends ScrollController {
  @override
  ScrollPosition createScrollPosition(
    ScrollPhysics physics,
    ScrollContext context,
    ScrollPosition? oldPosition,
  ) => _NativeScrollPosition(
    physics: physics,
    context: context,
    oldPosition: oldPosition,
  );
}

class _NativeScrollPosition extends ScrollPositionWithSingleContext {
  _NativeScrollPosition({
    required super.physics,
    required super.context,
    super.oldPosition,
  });
  void follow(double offset) {
    if (pixels != offset) forcePixels(offset);
  }
}
