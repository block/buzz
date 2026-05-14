import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:sprout_mobile/features/activity/activity_provider.dart';
import 'package:sprout_mobile/shared/relay/relay.dart';

void main() {
  const myPk = 'me';

  test('fetches agent job events and populates agentActivity', () async {
    final session = _FakeRelaySession(
      mentionEvents: [
        _event(id: 'mention-1', kind: 9, pubkey: 'alice', createdAt: 100),
      ],
      approvalEvents: [
        _event(id: 'approval-1', kind: 46010, pubkey: 'bot', createdAt: 200),
      ],
      agentJobEvents: [
        _event(id: 'job-1', kind: 43001, pubkey: 'agent', createdAt: 300),
        _event(id: 'job-2', kind: 43004, pubkey: 'agent', createdAt: 400),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final feed = await container.read(activityProvider.future);

    // Three parallel fetchHistory calls should have been issued.
    expect(session.historyFilters, hasLength(3));

    // Third filter should be for agent job kinds.
    final jobFilter = session.historyFilters[2];
    expect(jobFilter.kinds, containsAll([43001, 43003, 43004]));
    expect(jobFilter.tags['#p'], [myPk]);

    // agentActivity should contain the two job events.
    expect(feed.agentActivity, hasLength(2));
    expect(feed.agentActivity[0].id, 'job-2'); // newest first
    expect(feed.agentActivity[1].id, 'job-1');
    expect(feed.agentActivity[0].category, 'agent_activity');

    // mentions and needsAction should still work.
    expect(feed.mentions, hasLength(1));
    expect(feed.needsAction, hasLength(1));
  });

  test('returns empty agentActivity when no job events exist', () async {
    final session = _FakeRelaySession(
      mentionEvents: const [],
      approvalEvents: const [],
      agentJobEvents: const [],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final feed = await container.read(activityProvider.future);

    expect(feed.agentActivity, isEmpty);
    expect(feed.isEmpty, isTrue);
  });
}

NostrEvent _event({
  required String id,
  required int kind,
  required String pubkey,
  required int createdAt,
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: kind,
  tags: const [
    ['p', 'me'],
    ['h', 'ch1'],
  ],
  content: 'test content',
  sig: 'sig',
);

ProviderContainer _buildContainer({required _FakeRelaySession session}) {
  return ProviderContainer(
    overrides: [
      relayConfigProvider.overrideWith(() => _FakeRelayConfigNotifier()),
      relaySessionProvider.overrideWith(() => session),
      myPubkeyProvider.overrideWithValue('me'),
    ],
  );
}

class _FakeRelaySession extends RelaySessionNotifier {
  _FakeRelaySession({
    required this.mentionEvents,
    required this.approvalEvents,
    required this.agentJobEvents,
  });

  final List<NostrEvent> mentionEvents;
  final List<NostrEvent> approvalEvents;
  final List<NostrEvent> agentJobEvents;

  final List<NostrFilter> historyFilters = [];
  int _callIndex = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyFilters.add(filter);
    final idx = _callIndex++;
    switch (idx) {
      case 0:
        return mentionEvents;
      case 1:
        return approvalEvents;
      case 2:
        return agentJobEvents;
      default:
        return const [];
    }
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    return () {};
  }
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://fake.relay');
}
