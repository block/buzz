import 'package:flutter/foundation.dart';

import 'push_presentation_cache.dart';

/// Latest terminal export failure, retained across unrelated successful writes.
///
/// Unlike [pushPresentationCacheError], this records an export that could not
/// reach native storage. Retries are bounded and do not guarantee delivery.
final pushPresentationExportError = ValueNotifier<String?>(null);

/// Retries one export while its producer applies backpressure to new input.
///
/// Queue saturation retries the exact operation five times over 7.75 seconds.
/// Exhausted retries or a worker failure return false and record a terminal
/// error. Producers must serialize calls and retain dirty input upstream; this
/// helper does not queue, replace, or merge unverified event batches.
class PushPresentationExportRecovery {
  /// Exports once, recovering transient saturation without detached errors.
  Future<bool> export(Future<void> Function() operation) async {
    try {
      await operation();
      return true;
    } on PushPresentationExportQueueFull {
      // The producer retains this operation until recovery completes.
    } catch (error, stack) {
      return _failed(error, stack);
    }

    for (final milliseconds in [250, 500, 1000, 2000, 4000]) {
      await Future<void>.delayed(Duration(milliseconds: milliseconds));
      try {
        await operation();
        return true;
      } on PushPresentationExportQueueFull catch (error, stack) {
        if (milliseconds == 4000) return _failed(error, stack);
      } catch (error, stack) {
        return _failed(error, stack);
      }
    }
    return false;
  }

  bool _failed(Object error, StackTrace stack) {
    pushPresentationExportError.value = error.toString();
    debugPrint('Push presentation export could not be delivered: $error');
    debugPrintStack(stackTrace: stack);
    return false;
  }
}
