import 'dart:async';
import 'dart:math';

const channelQueryBatchSize = 100;
const liveSubscriptionMaxConcurrent = 4;
const liveSubscriptionStartInterval = Duration(milliseconds: 125);

typedef TaskDelay = Future<void> Function(Duration duration);

List<List<T>> chunkChannelQueryItems<T>(List<T> items) => [
  for (var start = 0; start < items.length; start += channelQueryBatchSize)
    items.sublist(start, min(start + channelQueryBatchSize, items.length)),
];

/// Runs tasks through one globally serialized admission chain.
///
/// The relay bills each REQ against a 50-per-five-second budget. A 125 ms
/// interval admits at most 40 requests in a half-open five-second window, while
/// the worker ceiling limits outstanding readiness waits when the relay stalls.
Future<void> runPacedTasks(
  List<Future<void> Function()> tasks, {
  required int maxConcurrent,
  required Duration startInterval,
  required bool Function() isCancelled,
  TaskDelay delay = defaultTaskDelay,
  void Function(Object error)? onError,
}) async {
  if (maxConcurrent < 1) throw ArgumentError.value(maxConcurrent);
  var nextTask = 0;
  var firstStart = true;
  Future<void> startPermit = Future.value();

  Future<bool> acquireStartPermit() async {
    if (firstStart) {
      firstStart = false;
    } else {
      startPermit = startPermit.then((_) => delay(startInterval));
      await startPermit;
    }
    return !isCancelled();
  }

  Future<void> worker() async {
    while (true) {
      final index = nextTask++;
      if (index >= tasks.length) return;
      if (!await acquireStartPermit()) return;
      try {
        await tasks[index]();
      } catch (error) {
        onError?.call(error);
      }
    }
  }

  await Future.wait([
    for (var i = 0; i < min(maxConcurrent, tasks.length); i++) worker(),
  ]);
}

Future<void> defaultTaskDelay(Duration duration) =>
    Future<void>.delayed(duration);
