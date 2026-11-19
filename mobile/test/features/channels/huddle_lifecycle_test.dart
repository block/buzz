import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test(
    'visible Huddle lifecycle follows relay participation and releases its read',
    () async {
      final relay = _LifecycleRelay();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relay)],
      );
      addTearDown(container.dispose);
      final provider = huddleLifecycleProvider('parent');
      final subscription = container.listen(provider, (_, _) {});
      expect(await container.read(provider.future), isEmpty);
      for (final kind in [
        EventKind.huddleParticipantJoined,
        EventKind.huddleParticipantLeft,
        EventKind.huddleEnded,
      ]) {
        relay.emit(
          NostrEvent(
            id: '$kind',
            pubkey: 'relay',
            createdAt: kind,
            kind: kind,
            tags: [
              ['h', 'parent'],
            ],
            content: '{}',
            sig: '',
          ),
        );
        await container.pump();
        expect((await container.read(provider.future)).last.kind, kind);
        expect(relay.listeners.length, 1);
      }
      subscription.close();
      await container.pump();
      expect(relay.listeners, isEmpty);
    },
  );
}

class _LifecycleRelay extends RelaySessionNotifier {
  final events = <NostrEvent>[];
  final listeners = <void Function(NostrEvent)>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void emit(NostrEvent event) {
    events.add(event);
    for (final listener in List.of(listeners)) {
      listener(event);
    }
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async =>
      events.where((event) => filter.kinds.contains(event.kind)).toList();

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    void listener(NostrEvent event) {
      if (filter.kinds.contains(event.kind)) onEvent(event);
    }

    listeners.add(listener);
    return () {
      listeners.remove(listener);
    };
  }
}
