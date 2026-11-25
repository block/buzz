import 'dart:math' as math;
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter/material.dart';
import '../../shared/theme/utility_surface_theme.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/widgets/agent_artwork_panel.dart';
import '../../shared/widgets/buzz_sheet_header.dart';
import '../../shared/widgets/buzz_navigation_metrics.dart';
import '../../shared/widgets/buzz_toast.dart';
import '../../shared/widgets/native_profile_sheet.dart';
import '../../shared/widgets/navigation_backdrop.dart';
import '../../shared/widgets/ios_navigation_bar.dart';

const _compactHeight = 340.0;
const _navigationHeight = 60.0;

class ProfileSheetLayout extends HookWidget {
  const ProfileSheetLayout({
    super.key,
    required this.hero,
    required this.body,
    this.avatarUrl,
    this.photoOpen = false,
    this.enlargedPhoto,
    this.portraitKey,
    this.onPhotoBack,
    this.status,
    this.embedded = false,
    this.sheetControls = true,
    this.agent = false,
    this.contentPadding = EdgeInsets.zero,
  });
  final Widget hero, body;
  final bool photoOpen;
  final Widget? enlargedPhoto, status;
  final GlobalKey? portraitKey;
  final VoidCallback? onPhotoBack;
  final String? avatarUrl;
  final bool embedded, sheetControls, agent;
  final EdgeInsets contentPadding;
  @override
  Widget build(BuildContext context) {
    final native = NativeProfileSheet.maybeOf(context);
    final showGrabber = !useValueListenable(buzzToastVisibleOf(context));
    final lastDimensions = useRef<(double, double, bool, bool)?>(null);
    final documentKey = useMemoized(GlobalKey.new);
    final nativeScroll = useMemoized(_NativeScrollController.new);
    useEffect(() => nativeScroll.dispose, [nativeScroll]);
    final active = (ModalRoute.isCurrentOf(context) ?? true) && !photoOpen;
    useEffect(() {
      void scroll() {
        if (native == null || !nativeScroll.hasClients) return;
        (nativeScroll.position as _NativeScrollPosition).follow(
          native.offset.value,
        );
      }

      native?.offset.addListener(scroll);
      return () => native?.offset.removeListener(scroll);
    }, [native?.offset, nativeScroll]);
    final statusKey = useMemoized(GlobalKey.new);
    final statusHeight = useState(0.0);
    final compactHeight = _compactHeight + statusHeight.value;
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
        final extra = bubble is RenderBox && bubble.hasSize
            ? bubble.size.height
            : 0.0;
        if (extra != statusHeight.value) statusHeight.value = extra;
        final document = documentKey.currentContext?.findRenderObject();
        if (native != null &&
            box is RenderBox &&
            document is RenderBox &&
            box.hasSize &&
            document.hasSize) {
          final dimensions = (
            _compactHeight + extra,
            _navigationHeight + document.size.height,
            active,
            showGrabber,
          );
          if (lastDimensions.value != dimensions) {
            lastDimensions.value = dimensions;
            native.setDimensions(
              dimensions.$1,
              dimensions.$2,
              dimensions.$3,
              showGrabber: showGrabber,
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
    final sectionTheme = utilitySurfaceThemeData(Theme.of(context));
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
    Widget panel(ScrollController? scroll, Widget header) => PopScope(
      canPop: !photoOpen,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && photoOpen) onPhotoBack?.call();
      },
      child: Theme(
        data: sectionTheme,
        child: ColoredBox(
          color: sectionTheme.scaffoldBackgroundColor,
          child: Stack(
            key: panelKey,
            children: [
              Positioned(
                top: 0,
                left: 0,
                right: 0,
                height: (sheetControls ? 60 : 0) + heroHeight.value,
                child: IgnorePointer(
                  child: AgentArtworkPanel(
                    key: const ValueKey('profile-fixed-backdrop'),
                    imageUrl: avatarUrl,
                    personProfile: !agent,
                    borderRadius: BorderRadius.zero,
                    child: const SizedBox.expand(),
                  ),
                ),
              ),
              fade(
                Padding(
                  padding: contentPadding,
                  child: SingleChildScrollView(
                    controller: scroll,
                    padding: EdgeInsets.only(
                      top: sheetControls ? _navigationHeight : 0,
                    ),
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
                                        sheetControls ? 8 : 44,
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
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ),
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
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: native.expanded
                ? null
                : (event) {
                    if (event.delta.dy < 0) native.expand();
                  },
            child: const BuzzSheetHeader(),
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
        child: panel(null, const BuzzSheetHeader()),
      );
    }
    return LayoutBuilder(
      builder: (context, box) {
        final height = box.maxHeight;
        final compact = (compactHeight / height).clamp(.12, 1.0);
        void settle(double velocity) {
          if (!detents.isAttached) return;
          final expand = velocity.abs() > 150
              ? velocity < 0
              : detents.size > (1 + compact) / 2;
          if (!expand && detents.size <= compact + .001 && velocity > 500) {
            Navigator.of(context).maybePop();
            return;
          }
          detents.animateTo(
            expand ? 1 : compact,
            duration: const Duration(milliseconds: 220),
            curve: Curves.easeOutCubic,
          );
        }

        final header = GestureDetector(
          behavior: HitTestBehavior.translucent,
          onVerticalDragUpdate: (event) {
            if (detents.isAttached) {
              detents.jumpTo(
                (detents.size - event.delta.dy / height).clamp(compact, 1.0),
              );
            }
          },
          onVerticalDragEnd: (event) =>
              settle(event.velocity.pixelsPerSecond.dy),
          child: BuzzSheetHeader(showDragHandle: showGrabber),
        );
        return DraggableScrollableSheet(
          controller: detents,
          initialChildSize: compact,
          minChildSize: compact,
          maxChildSize: 1,
          expand: false,
          snap: true,
          shouldCloseOnMinExtent: false,
          builder: (context, scroll) => panel(scroll, header),
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
