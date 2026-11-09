import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Marks visible message content that can start a reply drag.
class MessageReplyDragTarget extends SingleChildRenderObjectWidget {
  const MessageReplyDragTarget({super.key, required super.child});

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderMessageReplyDragTarget();
}

class _RenderMessageReplyDragTarget extends RenderProxyBox {
  @override
  bool hitTestSelf(Offset position) => true;
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
