import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// Ephemeral geometry owned by one channel or thread composer.
class ComposerSendOrigin {
  Rect? _rect;
  DateTime? _capturedAt;

  /// Captures only geometry; no draft or identity data is retained.
  void capture(GlobalKey key) {
    final box = key.currentContext?.findRenderObject();
    if (box is RenderBox && box.hasSize && box.attached) {
      _rect = box.localToGlobal(Offset.zero) & box.size;
      _capturedAt = DateTime.now();
    }
  }

  /// Stale captures cannot animate a message opened on later navigation.
  Rect? get recentRect =>
      _capturedAt != null &&
          DateTime.now().difference(_capturedAt!) < const Duration(seconds: 2)
      ? _rect
      : null;
}

/// Scope geometry independently for channel and thread composers.
final composerSendOriginProvider = Provider.autoDispose
    .family<ComposerSendOrigin, (String, String?)>(
      (ref, scope) => ComposerSendOrigin(),
    );
