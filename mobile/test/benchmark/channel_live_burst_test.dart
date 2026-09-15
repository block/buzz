// Opt-in synthetic diagnostic; see README.md for invocation and limitations.
import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test(
    'buffered live channel delivery',
    () async {
      for (final historySize in [50, 500, 2000]) {
        for (final burstSize in [50, 200, 1000]) {
          // Discard the first two runs to reduce debug/JIT warm-up effects.
          for (var run = 0; run < 6; run++) {
            await _measureBurst(historySize, burstSize, run);
          }
        }
      }
    },
    skip: !const bool.fromEnvironment('BUZZ_RUN_BENCHMARKS'),
    timeout: const Timeout(Duration(minutes: 3)),
  );
}

Future<void> _measureBurst(int historySize, int burstSize, int run) async {
  final session = _SyntheticSession(historySize);
  final container = ProviderContainer(
    overrides: [relaySessionProvider.overrideWith(() => session)],
  );
  try {
    final loaded = Completer<void>();
    container.listen(channelMessagesProvider(_channelId), (_, next) {
      if (!loaded.isCompleted && next.value?.length == historySize) {
        loaded.complete();
      }
    });
    await loaded.future.timeout(const Duration(seconds: 5));

    for (var index = 0; index < burstSize; index++) {
      session.debugHandleMessage([
        'EVENT',
        'l-1',
        _event('live-$index', historySize + index).toJson(),
      ]);
    }
    final tick = Completer<int>();
    final watch = Stopwatch()..start();
    Timer.run(() => tick.complete(watch.elapsedMicroseconds));
    session.debugFlushEventBuffer();
    final synchronousUs = watch.elapsedMicroseconds;
    final eventLoopDelayUs = await tick.future;

    expect(
      container.read(channelMessagesProvider(_channelId)).value!.length,
      historySize + burstSize,
    );
    if (run >= 2) {
      debugPrint(
        jsonEncode({
          'history': historySize,
          'burst': burstSize,
          'run': run - 2,
          'synchronous_us': synchronousUs,
          'event_loop_delay_us': eventLoopDelayUs,
        }),
      );
    }
  } finally {
    container.dispose();
    // The original 16 ms buffer timer remains armed after a debug flush.
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

const _channelId = '11111111-1111-4111-8111-111111111111';

NostrEvent _event(String id, int createdAt) => NostrEvent(
  id: id,
  pubkey: 'synthetic-author',
  createdAt: createdAt,
  kind: EventKind.streamMessageV2,
  tags: const [
    ['h', _channelId],
  ],
  content: 'Synthetic message',
  sig: 'synthetic-signature',
);

class _SyntheticSession extends RelaySessionNotifier {
  final int historySize;

  _SyntheticSession(this.historySize);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) {
    // Preserve the real subscription and buffer delivery implementation. This
    // fixture creates exactly one subscription, whose generated id is l-1.
    final ready = super.subscribe(filter, onEvent, onClosed: onClosed);
    debugHandleMessage(['EOSE', 'l-1']);
    return ready;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    // Model retained history in one synthetic page; real windows page by 50.
    for (var index = historySize - 1; index >= 0; index--)
      _event('history-$index', index),
    NostrEvent(
      id: 'bounds',
      pubkey: 'synthetic-relay',
      createdAt: 0,
      kind: EventKind.channelWindowBounds,
      tags: const [
        ['d', '$_channelId:head'],
      ],
      content: jsonEncode({'has_more': false, 'next_cursor': null}),
      sig: 'synthetic-signature',
    ),
  ];
}
