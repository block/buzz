import 'dart:async';

import 'package:buzz/shared/share_inbox/share_inbox_provider.dart';
import 'package:buzz/shared/share_inbox/shared_payload.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _payload = SharedPayload(
  id: 'p1',
  items: [SharedItem(kind: SharedItemKind.text, value: 'hi')],
);

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'surfaces a staged payload on build and releases it on consume',
    () async {
      final queue = <SharedPayload?>[_payload, null];
      final discarded = <String>[];
      final container = ProviderContainer(
        overrides: [
          sharedPayloadReaderProvider.overrideWithValue(
            () async => queue.isEmpty ? null : queue.removeAt(0),
          ),
          sharedPayloadDiscarderProvider.overrideWithValue((id) async {
            discarded.add(id);
          }),
        ],
      );
      addTearDown(container.dispose);

      expect(container.read(pendingSharedPayloadProvider), isNull);
      await _pump();
      expect(container.read(pendingSharedPayloadProvider), _payload);

      await container.read(pendingSharedPayloadProvider.notifier).consume('p1');
      expect(container.read(pendingSharedPayloadProvider), isNull);
      expect(discarded, ['p1']);
    },
  );

  test('re-checks the inbox when the app returns to the foreground', () async {
    binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    var reads = 0;
    final container = ProviderContainer(
      overrides: [
        sharedPayloadReaderProvider.overrideWithValue(() async {
          reads += 1;
          return reads == 2 ? _payload : null;
        }),
        sharedPayloadDiscarderProvider.overrideWithValue((_) async {}),
      ],
    );
    addTearDown(container.dispose);

    container.read(pendingSharedPayloadProvider);
    await _pump();
    expect(reads, 1);
    expect(container.read(pendingSharedPayloadProvider), isNull);

    // The test binding does not synthesize intermediate states.
    for (final state in const [
      AppLifecycleState.inactive,
      AppLifecycleState.hidden,
      AppLifecycleState.paused,
      AppLifecycleState.hidden,
      AppLifecycleState.inactive,
      AppLifecycleState.resumed,
    ]) {
      binding.handleAppLifecycleStateChanged(state);
    }
    await _pump();
    expect(reads, 2);
    expect(container.read(pendingSharedPayloadProvider), _payload);
  });

  test('releasing an older payload keeps a newer one current', () async {
    const newer = SharedPayload(
      id: 'p2',
      items: [SharedItem(kind: SharedItemKind.text, value: 'later')],
    );
    final queue = <SharedPayload?>[_payload, newer];
    final discarded = <String>[];
    final container = ProviderContainer(
      overrides: [
        sharedPayloadReaderProvider.overrideWithValue(
          () async => queue.isEmpty ? null : queue.removeAt(0),
        ),
        sharedPayloadDiscarderProvider.overrideWithValue((id) async {
          discarded.add(id);
        }),
      ],
    );
    addTearDown(container.dispose);

    container.read(pendingSharedPayloadProvider);
    await _pump();
    final notifier = container.read(pendingSharedPayloadProvider.notifier);
    await notifier.sync();
    expect(container.read(pendingSharedPayloadProvider), newer);

    await notifier.consume(_payload.id);
    expect(discarded, ['p1']);
    expect(container.read(pendingSharedPayloadProvider), newer);
  });

  test('coalesces overlapping sync calls', () async {
    final gate = Completer<SharedPayload?>();
    var reads = 0;
    final container = ProviderContainer(
      overrides: [
        sharedPayloadReaderProvider.overrideWithValue(() {
          reads += 1;
          return gate.future;
        }),
        sharedPayloadDiscarderProvider.overrideWithValue((_) async {}),
      ],
    );
    addTearDown(container.dispose);

    final notifier = container.read(pendingSharedPayloadProvider.notifier);
    final first = notifier.sync();
    final second = notifier.sync();
    expect(reads, 1);
    gate.complete(_payload);
    await Future.wait([first, second]);
    expect(container.read(pendingSharedPayloadProvider), _payload);
  });
}

Future<void> _pump() => Future<void>.delayed(Duration.zero);
