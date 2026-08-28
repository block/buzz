import 'dart:async';

import 'package:buzz/features/channels/channel_sync.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('query chunks preserve exact 100 boundary', () {
    expect(
      chunkChannelQueryItems(List.generate(100, (i) => i)).map((b) => b.length),
      [100],
    );
    expect(
      chunkChannelQueryItems(List.generate(129, (i) => i)).map((b) => b.length),
      [100, 29],
    );
  });

  test('paced admission captures one shared 125 ms interval', () async {
    final delays = <Duration>[];
    final starts = <int>[];
    await runPacedTasks(
      [for (var i = 0; i < 6; i++) () async => starts.add(i)],
      maxConcurrent: liveSubscriptionMaxConcurrent,
      startInterval: liveSubscriptionStartInterval,
      isCancelled: () => false,
      delay: (duration) async => delays.add(duration),
    );
    expect(starts.toSet(), {0, 1, 2, 3, 4, 5});
    expect(delays, List.filled(5, const Duration(milliseconds: 125)));
  });

  test('start permits are globally serialized across workers', () async {
    final pendingDelays = <Completer<void>>[];
    var activeDelays = 0;
    var peakActiveDelays = 0;
    final run = runPacedTasks(
      [for (var i = 0; i < 6; i++) () async {}],
      maxConcurrent: liveSubscriptionMaxConcurrent,
      startInterval: liveSubscriptionStartInterval,
      isCancelled: () => false,
      delay: (_) {
        activeDelays++;
        peakActiveDelays = activeDelays > peakActiveDelays
            ? activeDelays
            : peakActiveDelays;
        final pending = Completer<void>();
        pendingDelays.add(pending);
        return pending.future.whenComplete(() => activeDelays--);
      },
    );

    for (var completed = 0; completed < 5; completed++) {
      await _waitUntil(() => pendingDelays.length == completed + 1);
      expect(activeDelays, 1);
      pendingDelays[completed].complete();
    }
    await run;
    expect(peakActiveDelays, 1);
  });

  test('slow tasks never exceed four in flight', () async {
    var inFlight = 0;
    var peakInFlight = 0;
    final releases = <Completer<void>>[];
    final run = runPacedTasks(
      [
        for (var i = 0; i < 12; i++)
          () async {
            inFlight++;
            if (inFlight > peakInFlight) peakInFlight = inFlight;
            final release = Completer<void>();
            releases.add(release);
            await release.future;
            inFlight--;
          },
      ],
      maxConcurrent: liveSubscriptionMaxConcurrent,
      startInterval: liveSubscriptionStartInterval,
      isCancelled: () => false,
      delay: (_) async {},
    );
    await _waitUntil(() => releases.length >= 4);
    expect(peakInFlight, 4);
    while (releases.length < 12) {
      final count = releases.length;
      for (final release in releases.take(count).where((c) => !c.isCompleted)) {
        release.complete();
      }
      await _waitUntil(() => releases.length > count || releases.length == 12);
    }
    for (final release in releases.where((c) => !c.isCompleted)) {
      release.complete();
    }
    await run;
    expect(peakInFlight, 4);
  });
}

Future<void> _waitUntil(bool Function() predicate) async {
  for (var i = 0; i < 100; i++) {
    if (predicate()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('condition did not become true');
}
