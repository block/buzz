import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

const _iosMessageLongPressDuration = Duration(milliseconds: 200);
const _maxMessageSnapshotDimension = 2048.0;

double _messageSnapshotPixelRatio(Size size, double devicePixelRatio) {
  final longestSide = size.longestSide;
  if (!longestSide.isFinite || longestSide <= 0) return devicePixelRatio;
  return math.min(devicePixelRatio, _maxMessageSnapshotDimension / longestSide);
}

/// Geometry and snapshot controls for a completed message long press.
class MessageLongPressDetails {
  final Rect anchorRect;
  final Future<ui.Image> Function() captureSnapshot;
  final ValueChanged<bool> setSourceHidden;

  const MessageLongPressDetails({
    required this.anchorRect,
    required this.captureSnapshot,
    required this.setSourceHidden,
  });
}

/// An [InkWell] whose long press is observed above interactive descendants.
class MessageLongPressInkWell extends StatelessWidget {
  final VoidCallback? onTap;
  final ValueChanged<Rect>? onLongPress;
  final ValueChanged<MessageLongPressDetails>? onLongPressDetails;
  final BorderRadius? borderRadius;
  final Color? highlightColor;
  final GlobalKey? snapshotKey;
  final Widget child;

  const MessageLongPressInkWell({
    super.key,
    this.onTap,
    this.onLongPress,
    this.onLongPressDetails,
    this.borderRadius,
    this.highlightColor,
    this.snapshotKey,
    required this.child,
  }) : assert(onLongPress != null || onLongPressDetails != null);

  @override
  Widget build(BuildContext context) {
    return _MessageLongPressRegion(
      onTap: onTap,
      onLongPress: onLongPress,
      onLongPressDetails: onLongPressDetails,
      borderRadius: borderRadius,
      highlightColor: highlightColor,
      externalSnapshotKey: snapshotKey,
      child: child,
    );
  }
}

/// Detects a message long press through Flutter's gesture arena.
///
/// Links, media, reactions, and other nested controls keep their normal tap
/// gestures, while a completed hold wins over descendant taps.
class _MessageLongPressRegion extends HookWidget {
  final VoidCallback? onTap;
  final ValueChanged<Rect>? onLongPress;
  final ValueChanged<MessageLongPressDetails>? onLongPressDetails;
  final BorderRadius? borderRadius;
  final Color? highlightColor;
  final GlobalKey? externalSnapshotKey;
  final Widget child;

  const _MessageLongPressRegion({
    required this.onTap,
    required this.onLongPress,
    required this.onLongPressDetails,
    required this.borderRadius,
    required this.highlightColor,
    required this.externalSnapshotKey,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final fallbackSnapshotKey = useMemoized(GlobalKey.new, const []);
    final snapshotKey = externalSnapshotKey ?? fallbackSnapshotKey;
    final sourceHidden = useState(false);

    void recognize() {
      final renderObject = snapshotKey.currentContext?.findRenderObject();
      if (renderObject is! RenderRepaintBoundary || !renderObject.hasSize) {
        return;
      }
      final anchorRect =
          renderObject.localToGlobal(Offset.zero) & renderObject.size;

      final detailsCallback = onLongPressDetails;
      if (detailsCallback == null) {
        onLongPress?.call(anchorRect);
        return;
      }
      final maxSnapshotPixelRatio = math.min(
        MediaQuery.devicePixelRatioOf(context),
        2.0,
      );

      Future<ui.Image> captureSnapshot() async {
        RenderRepaintBoundary? boundary;
        final renderObject = snapshotKey.currentContext?.findRenderObject();
        if (renderObject is RenderRepaintBoundary && renderObject.hasSize) {
          boundary = renderObject;
        }
        if (boundary == null) {
          throw StateError('Message snapshot is unavailable');
        }
        final snapshotPixelRatio = _messageSnapshotPixelRatio(
          boundary.size,
          maxSnapshotPixelRatio,
        );
        try {
          return await boundary.toImage(pixelRatio: snapshotPixelRatio);
        } catch (_) {
          await WidgetsBinding.instance.endOfFrame;
          final retryBoundary = snapshotKey.currentContext?.findRenderObject();
          if (retryBoundary is! RenderRepaintBoundary ||
              !retryBoundary.hasSize) {
            rethrow;
          }
          try {
            return await retryBoundary.toImage(
              pixelRatio: _messageSnapshotPixelRatio(
                retryBoundary.size,
                maxSnapshotPixelRatio,
              ),
            );
          } catch (_) {
            if (snapshotPixelRatio <= 1) rethrow;
            return retryBoundary.toImage(
              pixelRatio: _messageSnapshotPixelRatio(retryBoundary.size, 1),
            );
          }
        }
      }

      void setSourceHidden(bool hidden) {
        if (!context.mounted) return;
        sourceHidden.value = hidden;
      }

      detailsCallback(
        MessageLongPressDetails(
          anchorRect: anchorRect,
          captureSnapshot: captureSnapshot,
          setSourceHidden: setSourceHidden,
        ),
      );
    }

    return Semantics(
      onLongPress: recognize,
      child: RawGestureDetector(
        behavior: HitTestBehavior.translucent,
        gestures: {
          LongPressGestureRecognizer:
              GestureRecognizerFactoryWithHandlers<LongPressGestureRecognizer>(
                () => LongPressGestureRecognizer(
                  duration: defaultTargetPlatform == TargetPlatform.iOS
                      ? _iosMessageLongPressDuration
                      : null,
                ),
                (recognizer) {
                  recognizer.onLongPressStart = (_) => recognize();
                },
              ),
        },
        child: InkWell(
          onTap: onTap,
          borderRadius: borderRadius,
          highlightColor: highlightColor,
          child: Opacity(
            opacity: sourceHidden.value ? 0 : 1,
            child: externalSnapshotKey == null
                ? RepaintBoundary(key: snapshotKey, child: child)
                : child,
          ),
        ),
      ),
    );
  }
}
