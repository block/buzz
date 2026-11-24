import 'dart:async';

import 'push_wake_resolver.dart';

/// Runs wakes one at a time in this process. A wake that arrives while one is
/// running does not start a second run: it sets a rerun flag, and the running
/// wake looks again when it finishes, so a message that arrived after its
/// fetch is still found. Every caller waiting gets one result, with the
/// notifications of every pass.
///
/// All passes of a run share one [budget], which stays inside the native
/// watchdog, so wakes that keep arriving can't extend a run past it. A rerun
/// starts only when at least [minimumPass] of the budget is left; otherwise the
/// run returns what it has, and a message it skipped is found by the next
/// wake, which fetches from before the newest shown message.
class PushWakeRunner {
  PushWakeRunner(
    this._resolve, {
    this.budget = const Duration(seconds: 5),
    this.minimumPass = const Duration(seconds: 1),
  });

  /// Runs one pass within the time it is given.
  final Future<PushWakeResult> Function(Duration budget) _resolve;

  /// The longest a run may take, over all its passes.
  final Duration budget;

  /// The least time worth starting a rerun with.
  final Duration minimumPass;

  Future<PushWakeResult>? _running;
  bool _rerun = false;

  /// Runs a wake, or joins the one in progress. [remaining] is the time the
  /// caller has left, when it started counting before this call (the native
  /// handler's watchdog runs from the push, not from when this engine became
  /// ready). A new run takes the shorter of it and [budget]; a joined call
  /// changes nothing, since the run in progress already ends inside it.
  Future<PushWakeResult> wake({Duration? remaining}) {
    final running = _running;
    if (running != null) {
      _rerun = true;
      return running;
    }
    final runBudget = remaining != null && remaining < budget
        ? remaining
        : budget;
    return _running = _loop(runBudget).whenComplete(() => _running = null);
  }

  Future<PushWakeResult> _loop(Duration budget) async {
    final elapsed = Stopwatch()..start();
    final notifications = <PushWakeNotification>[];
    final failed = <String>{};
    var first = true;
    var interrupted = false;
    var lastInterrupted = false;
    do {
      _rerun = false;
      final PushWakeResult result;
      try {
        result = await _resolve(budget - elapsed.elapsed);
      } on Object {
        // A rerun that fails must not discard what earlier passes claimed,
        // since those messages are already recorded as shown. The run is
        // still incomplete: the rerun was for a message it never looked for,
        // so the caller falls back to its generic notification when nothing
        // else was found.
        if (first) rethrow;
        interrupted = true;
        break;
      }
      first = false;
      notifications.addAll(result.notifications);
      // Only the last pass says what is still unread, because a community
      // that failed once may have been read by the next, and a pass that ran
      // out of time may have been followed by one that finished.
      lastInterrupted = result.interrupted;
      failed
        ..clear()
        ..addAll(result.failedCommunityIds);
    } while (_rerun && budget - elapsed.elapsed >= minimumPass);
    return PushWakeResult(
      notifications: capPushWakeNotifications(notifications),
      failedCommunityIds: failed.toList()..sort(),
      // A wake that arrived during the last pass and got no rerun for lack of
      // budget was never looked up, so the caller must fall back for it.
      interrupted: interrupted || lastInterrupted || _rerun,
    );
  }
}
