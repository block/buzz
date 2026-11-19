import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test(
    'visible Huddle lifecycle follows relay participation and releases its read',
    () async {
      final relay = _LifecycleRelay();
      NostrEvent event(int kind, int time) => NostrEvent(
        id: '$kind-$time',
        pubkey: 'relay',
        createdAt: time,
        kind: kind,
        tags: [
          ['h', 'parent'],
        ],
        content: '{}',
        sig: '',
      );
      relay.events.add(event(EventKind.huddleStarted, 1));
      relay.events.addAll(
        List.generate(
          250,
          (i) => event(EventKind.huddleParticipantJoined, i + 2),
        ),
      );
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relay)],
      );
      addTearDown(container.dispose);
      final provider = huddleLifecycleProvider('parent');
      final subscription = container.listen(provider, (_, _) {});
      final initial = await container.read(provider.future);
      expect(initial.any((e) => e.kind == EventKind.huddleStarted), isTrue);
      expect(initial, hasLength(201));
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
        expect(relay.historyReads, 2);
      }
      relay.statusListener!(RelaySubscriptionStatus.ready);
      await container.pump();
      expect(relay.historyReads, 4);
      expect(
        container
            .read(provider)
            .requireValue
            .any((e) => e.kind == EventKind.huddleEnded),
        isTrue,
      );
      expect(relay.listeners, hasLength(1));
      subscription.close();
      await container.pump();
      expect(relay.listeners, isEmpty);
    },
  );

  test('a failed participation read still exposes the active start', () async {
    final relay = _LifecycleRelay()
      ..failingKinds = {EventKind.huddleParticipantJoined};
    relay.events.add(
      NostrEvent(
        id: 'start',
        pubkey: 'relay',
        createdAt: 1,
        kind: EventKind.huddleStarted,
        tags: [
          ['h', 'parent'],
        ],
        content: '{}',
        sig: '',
      ),
    );
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relay)],
    );
    addTearDown(container.dispose);
    final provider = huddleLifecycleProvider('parent');
    container.listen(provider, (_, _) {});
    final initial = await container.read(provider.future);
    expect(initial.single.id, 'start');
    expect(relay.historyReads, 2);
  });
}

class _LifecycleRelay extends RelaySessionNotifier {
  int historyReads = 0;
  Set<int> failingKinds = const {};
  void Function(RelaySubscriptionStatus)? statusListener;
  final events = <NostrEvent>[];
  final listeners = <void Function(NostrEvent)>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void emit(NostrEvent event) {
    for (final listener in List.of(listeners)) {
      listener(event);
    }
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyReads++;
    if (filter.kinds.any(failingKinds.contains)) {
      throw Exception('history timed out');
    }
    final matching =
        events.where((event) => filter.kinds.contains(event.kind)).toList()
          ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
    return matching.take(filter.limit).toList();
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
    bool resumeSince = true,
  }) async {
    void listener(NostrEvent event) {
      if (filter.kinds.contains(event.kind)) onEvent(event);
    }

    statusListener = onStatusChanged;
    listeners.add(listener);
    return () {
      listeners.remove(listener);
    };
  }
}
