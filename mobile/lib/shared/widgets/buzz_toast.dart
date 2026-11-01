import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import '../toast_haptic.dart';
import 'toast_capsule_content.dart';

export '../toast_haptic.dart' show ToastKind;

part 'buzz_toast_island.dart';
part 'reconnect_capsule.dart';

final _toastCoordinators = Expando<_ToastCoordinator>();

_ToastCoordinator _toastCoordinator(BuildContext context) {
  final overlay = Overlay.of(context, rootOverlay: true);
  return _toastCoordinators[overlay] ??= _ToastCoordinator();
}

class _ToastCoordinator extends ValueNotifier<bool> {
  _ToastCoordinator() : super(false);

  _ToastHandle? active;
}

/// Shows brief feedback above routes and modal sheets.
/// A new notice replaces the current one instead of building a stale queue.
/// Supported portrait iPhones morph from the Dynamic Island automatically.
void showBuzzToast(
  BuildContext context,
  String message, {
  ToastKind kind = ToastKind.generic,
  VoidCallback? onRetry,
  Rect? islandOrigin,
  double islandSpeed = 0.75,
}) {
  final overlay = Overlay.of(context, rootOverlay: true);
  final coordinator = _toastCoordinator(context);
  coordinator.active?.remove();
  late final _ToastHandle handle;
  final entry = OverlayEntry(
    builder: (_) => _ToastLayer(
      child: _ToastBubble(
        message: message,
        islandOrigin: islandOrigin,
        islandSpeed: islandSpeed,
        onDismiss: handle.remove,
        onRetry: onRetry == null
            ? null
            : () {
                handle.remove();
                if (context.mounted) onRetry();
              },
      ),
    ),
  );
  handle = _ToastHandle(entry, () {
    if (identical(coordinator.active, handle)) {
      coordinator.active = null;
      coordinator.value = false;
    }
  });
  coordinator.active = handle;
  coordinator.value = true;
  overlay.insert(entry);
  unawaited(toastHaptic(kind));
}

// iOS applies a platform view's rounded clip to the Flutter overlay slice
// painted after it. Start an unclipped native slice so a sheet's close button
// cannot clip this toast back underneath earlier native navigation views.
class _ToastLayer extends HookWidget {
  const _ToastLayer({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final ready = useState(false);
    if (defaultTargetPlatform != TargetPlatform.iOS) return child;
    return Stack(
      fit: StackFit.expand,
      children: [
        // A single point in the window corner establishes the composition
        // slice without a native hit-test surface over any app controls.
        Positioned(
          top: 0,
          left: 0,
          width: 1,
          height: 1,
          child: ExcludeSemantics(
            child: UiKitView(
              viewType: 'buzz/toast_layer',
              hitTestBehavior: PlatformViewHitTestBehavior.transparent,
              onPlatformViewCreated: (_) => ready.value = true,
            ),
          ),
        ),
        if (ready.value) child,
      ],
    );
  }
}

class _ToastHandle {
  _ToastHandle(this.entry, this.onRemove);

  final OverlayEntry entry;
  final VoidCallback onRemove;
  bool removed = false;

  void remove() {
    if (removed) return;
    removed = true;
    entry.remove();
    entry.dispose();
    onRemove();
  }
}

class _ToastBubble extends HookWidget {
  const _ToastBubble({
    required this.message,
    required this.onDismiss,
    this.onRetry,
    this.islandOrigin,
    this.islandSpeed = 0.75,
    this.visible = true,
    this.persistent = false,
    this.actionLabel = 'Retry',
  });

  final String message;
  final VoidCallback? onRetry;
  final VoidCallback onDismiss;
  final Rect? islandOrigin;
  final double islandSpeed;
  final bool visible;
  final bool persistent;
  final String actionLabel;

  @override
  Widget build(BuildContext context) {
    final origin = islandOrigin ?? _toastIslandOrigin(context);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final accessible = MediaQuery.accessibleNavigationOf(context);
    final program = useFuture(
      useMemoized(
        () => origin != null && !reduceMotion ? _loadIslandProgram() : null,
        [origin != null, reduceMotion],
      ),
    );
    final shader = useMemoized(() => program.data?.fragmentShader(), [
      program.data,
    ]);
    useEffect(
      () =>
          () => shader?.dispose(),
      [shader],
    );
    final waitingForIsland =
        origin != null &&
        !reduceMotion &&
        program.connectionState != ConnectionState.done;
    final island = reduceMotion || shader == null ? null : origin;
    final speed = island == null ? 1.0 : islandSpeed.clamp(1 / 3, 1.0);
    final entranceMs = (280 / speed).round();
    final controller = useAnimationController(
      duration: Duration(milliseconds: entranceMs),
      reverseDuration: Duration(milliseconds: (240 / speed).round()),
    );
    useEffect(
      () {
        if (waitingForIsland) return null;
        if (persistent) {
          if (reduceMotion) {
            controller.value = visible ? 1 : 0;
          } else if (visible) {
            controller.forward();
          } else {
            controller.reverse();
          }
          return null;
        }
        if (reduceMotion) {
          controller.value = 1;
        } else {
          controller.forward();
        }
        final timer = accessible && onRetry != null
            ? null
            : Timer(
                Duration(
                  milliseconds:
                      (accessible || onRetry != null ? 6000 : 3000) +
                      (reduceMotion ? 0 : entranceMs),
                ),
                () async {
                  if (!reduceMotion) {
                    try {
                      await controller.reverse().orCancel;
                    } on TickerCanceled {
                      return;
                    }
                  }
                  onDismiss();
                },
              );
        return () => timer?.cancel();
      },
      [
        controller,
        reduceMotion,
        accessible,
        persistent,
        visible,
        waitingForIsland,
      ],
    );
    final value = useAnimation(controller);
    if (waitingForIsland || (persistent && value == 0)) {
      return const SizedBox.shrink();
    }
    final exiting = controller.status == AnimationStatus.reverse;
    // The exit collapses immediately while upward travel accelerates later.
    // Reversing the entry width curve leaves the text fully exposed too long.
    final progress = exiting
        ? Curves.easeOutQuad.transform(value)
        : const Cubic(0.2, 1.1, 0.4, 1).transform(value);
    final reveal = exiting
        ? Curves.easeInCubic.transform(value)
        : Curves.easeOutCubic.transform(value);
    final contentOpacity = exiting
        ? value * value
        : island == null
        ? 1.0
        : const Interval(0.25, 0.8).transform(value);
    final top = MediaQuery.viewPaddingOf(context).top + Grid.xxs;
    // Follow the system preference even when the current route overrides Theme.
    final dark = MediaQuery.platformBrightnessOf(context) == Brightness.dark;
    final background = island != null
        ? Colors.black
        : dark
        ? darkColorScheme.surfaceContainer
        : lightColorScheme.inverseSurface;
    final foreground = dark
        ? darkColorScheme.onSurface
        : lightColorScheme.onInverseSurface;
    return Positioned(
      top: top,
      left: Grid.gutter,
      right: Grid.gutter,
      child: IgnorePointer(
        ignoring: onRetry == null || exiting,
        child: Align(
          alignment: Alignment.topCenter,
          child: FractionalTranslation(
            translation: Offset(
              0,
              -(island == null ? 1 : 0.5) * (1 - progress),
            ),
            child: Transform.translate(
              offset: Offset(
                0,
                ((island?.center.dy ?? 0) - top) * (1 - progress),
              ),
              child: Semantics(
                liveRegion: true,
                onDismiss: persistent ? null : onDismiss,
                // Lay text out once at its full width. Only the capsule's
                // painted bounds expand, revealing rather than scaling text.
                child: CustomPaint(
                  painter: island == null || shader == null
                      ? null
                      : _ToastIslandSurface(
                          shader: shader,
                          reveal: reveal,
                          pixelRatio: MediaQuery.devicePixelRatioOf(context),
                          island: island,
                          top: top,
                          progress: progress,
                        ),
                  child: PhysicalShape(
                    clipper: _ToastRevealClipper(reveal),
                    clipBehavior: Clip.antiAlias,
                    color: island != null && shader != null
                        ? Colors.transparent
                        : background,
                    elevation: island != null && shader != null ? 0 : 4,
                    shadowColor: Colors.black.withValues(alpha: 0.18),
                    child: Opacity(
                      opacity: contentOpacity,
                      child: ToastCapsuleContent(
                        message: message,
                        foreground: foreground,
                        actionLabel: actionLabel,
                        onAction: onRetry,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ToastRevealClipper extends CustomClipper<Path> {
  const _ToastRevealClipper(this.progress);

  final double progress;

  @override
  Path getClip(Size size) {
    final collapsedWidth = size.height.clamp(0.0, size.width);
    final width = collapsedWidth + (size.width - collapsedWidth) * progress;
    return RoundedSuperellipseBorder(
      borderRadius: BorderRadius.circular(size.height / 2),
    ).getOuterPath(
      Rect.fromCenter(
        center: size.center(Offset.zero),
        width: width,
        height: size.height,
      ),
    );
  }

  @override
  bool shouldReclip(_ToastRevealClipper oldClipper) =>
      oldClipper.progress != progress;
}
