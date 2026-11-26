import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Marks a painted part of a message that opens its long-press actions but
/// cannot start a reply drag: the author line, footer and similar.
///
/// Wrap the part's shrink-wrapped extent, not a full-width row, so the empty
/// space beside it stays inert.
class MessageLongPressTarget extends SingleChildRenderObjectWidget {
  const MessageLongPressTarget({super.key, required super.child});

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderMessageLongPressTarget();
}

class _RenderMessageLongPressTarget extends RenderProxyBox {
  @override
  bool hitTestSelf(Offset position) => true;
}

/// Marks visible message content that can start a reply drag and, unless
/// [opensActions] is false, open the message's long-press actions.
class MessageReplyDragTarget extends SingleChildRenderObjectWidget {
  const MessageReplyDragTarget({
    super.key,
    this.opensActions = true,
    required super.child,
  });

  /// Whether a long press anywhere on this target opens the message's actions.
  ///
  /// Pass false when the target is wider than what it paints, and mark the
  /// painted part with a [MessageLongPressTarget] instead.
  final bool opensActions;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderMessageReplyDragTarget(opensActions);

  @override
  void updateRenderObject(
    BuildContext context,
    covariant RenderObject renderObject,
  ) {
    (renderObject as _RenderMessageReplyDragTarget).opensActions = opensActions;
  }
}

class _RenderMessageReplyDragTarget extends RenderProxyBox {
  _RenderMessageReplyDragTarget(this.opensActions);

  bool opensActions;

  @override
  bool hitTestSelf(Offset position) => true;
}

/// Whether [globalPosition] lands on a painted part of the message under
/// [region], rather than on the empty row space around those parts.
bool messageLongPressTargetAt(RenderObject? region, Offset globalPosition) {
  if (region is! RenderBox || !region.hasSize) return false;
  final result = BoxHitTestResult();
  region.hitTest(result, position: region.globalToLocal(globalPosition));
  return result.path.any((entry) {
    final target = entry.target;
    return target is _RenderMessageLongPressTarget ||
        target is _RenderMessageReplyDragTarget && target.opensActions;
  });
}

/// Tests painted message regions rather than the full-width content layout.
Rect? messageReplyDragTargetBounds(
  GlobalKey contentKey,
  Offset globalPosition,
) {
  final box = contentKey.currentContext?.findRenderObject();
  if (box is! RenderBox || !box.hasSize) return null;
  final result = BoxHitTestResult();
  box.hitTest(result, position: box.globalToLocal(globalPosition));
  for (final entry in result.path) {
    final target = entry.target;
    if (target is _RenderMessageReplyDragTarget) {
      return target.localToGlobal(Offset.zero) & target.size;
    }
  }
  return null;
}
