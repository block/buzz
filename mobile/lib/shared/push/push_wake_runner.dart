import 'dart:async';

import 'push_wake_resolver.dart';

/// Runs wakes one at a time in this process. A wake that arrives while one is
/// running does not start a second run: it sets a rerun flag, and the running
/// wake looks again when it finishes, so a message that arrived after its
/// fetch is still found. Every caller waiting gets one result, with the
/// notifications of every pass.
class PushWakeRunner {
  PushWakeRunner(this._resolve);

  final Future<PushWakeResult> Function() _resolve;
  Future<PushWakeResult>? _running;
  bool _rerun = false;

  Future<PushWakeResult> wake() {
    final running = _running;
    if (running != null) {
      _rerun = true;
      return running;
    }
    return _running = _loop().whenComplete(() => _running = null);
  }

  Future<PushWakeResult> _loop() async {
    final notifications = <PushWakeNotification>[];
    final failed = <String>{};
    do {
      _rerun = false;
      final result = await _resolve();
      notifications.addAll(result.notifications);
      // Only the last pass says what is still unread, because a community
      // that failed once may have been read by the next.
      failed
        ..clear()
        ..addAll(result.failedCommunityIds);
    } while (_rerun);
    return PushWakeResult(
      notifications: capPushWakeNotifications(notifications),
      failedCommunityIds: failed.toList()..sort(),
    );
  }
}
