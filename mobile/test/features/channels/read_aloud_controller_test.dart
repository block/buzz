import 'dart:async';

import 'package:buzz/features/channels/read_aloud/read_aloud_controller.dart';
import 'package:buzz/features/channels/read_aloud/read_aloud_engine.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _FakeEngine implements ReadAloudEngine {
  final spoken = <String>[];
  var stops = 0;
  final pending = <Completer<void>>[];

  @override
  Future<void> speak(String text) {
    spoken.add(text);
    final done = Completer<void>();
    pending.add(done);
    return done.future;
  }

  @override
  Future<void> stop() async {
    stops++;
    for (final done in pending) {
      if (!done.isCompleted) done.complete();
    }
  }

  void finish() {
    for (final done in pending) {
      if (!done.isCompleted) done.complete();
    }
  }
}

void main() {
  test('speaks cleaned text and stops on a second toggle', () async {
    final engine = _FakeEngine();
    final container = ProviderContainer(
      overrides: [readAloudEngineProvider.overrideWithValue(engine)],
    );
    addTearDown(container.dispose);
    final notifier = container.read(readAloudControllerProvider.notifier);

    final started = notifier.toggle(
      messageId: 'msg-1',
      content: '**Hello** `world`',
    );
    await Future<void>.delayed(Duration.zero);
    expect(container.read(readAloudControllerProvider), 'msg-1');
    expect(engine.spoken, ['Hello world']);

    final stopped = await notifier.toggle(
      messageId: 'msg-1',
      content: 'ignored',
    );
    expect(stopped, ReadAloudOutcome.stopped);
    expect(container.read(readAloudControllerProvider), isNull);
    expect(engine.stops, greaterThan(0));

    engine.finish();
    expect(await started, ReadAloudOutcome.finished);
    expect(container.read(readAloudControllerProvider), isNull);
  });

  test('does not start the engine when cleaning leaves nothing', () async {
    final engine = _FakeEngine();
    final container = ProviderContainer(
      overrides: [readAloudEngineProvider.overrideWithValue(engine)],
    );
    addTearDown(container.dispose);

    final outcome = await container
        .read(readAloudControllerProvider.notifier)
        .toggle(messageId: 'msg-1', content: '.');
    expect(outcome, ReadAloudOutcome.nothingToRead);
    expect(engine.spoken, isEmpty);
    expect(container.read(readAloudControllerProvider), isNull);
  });

  test('a new message replaces the one in progress', () async {
    final engine = _FakeEngine();
    final container = ProviderContainer(
      overrides: [readAloudEngineProvider.overrideWithValue(engine)],
    );
    addTearDown(container.dispose);
    final notifier = container.read(readAloudControllerProvider.notifier);

    final first = notifier.toggle(messageId: 'msg-1', content: 'first');
    await Future<void>.delayed(Duration.zero);
    final second = notifier.toggle(messageId: 'msg-2', content: 'second');
    await Future<void>.delayed(Duration.zero);

    expect(container.read(readAloudControllerProvider), 'msg-2');
    expect(engine.spoken, ['first', 'second']);
    engine.finish();
    await Future.wait([first, second]);
    expect(container.read(readAloudControllerProvider), isNull);
  });
}
