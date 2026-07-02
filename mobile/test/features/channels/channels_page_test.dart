import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';

void main() {
  Widget buildTestable({required List<Override> overrides}) {
    return ProviderScope(
      overrides: [
        // Provide a fake profile and presence so the avatar doesn't hit the network.
        profileProvider.overrideWith(() => _FakeProfileNotifier()),
        presenceProvider.overrideWith(() => _FakePresenceNotifier()),
        ...overrides,
      ],
      child: MaterialApp(theme: AppTheme.light(), home: const ChannelsPage()),
    );
  }

  final testChannels = [
    Channel(
      id: '1',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: 'General discussion',
      createdBy: 'abc',
      createdAt: DateTime(2025),
      memberCount: 10,
      isMember: true,
    ),
    Channel(
      id: '2',
      name: 'design-forum',
      channelType: 'forum',
      visibility: 'open',
      description: 'Discuss designs',
      createdBy: 'abc',
      createdAt: DateTime(2025),
      memberCount: 3,
      isMember: true,
    ),
    Channel(
      id: '3',
      name: 'DM',
      channelType: 'dm',
      visibility: 'open',
      description: 'Direct message',
      createdBy: 'abc',
      createdAt: DateTime(2025),
      memberCount: 2,
      participants: const ['Test', 'Alice'],
      participantPubkeys: const ['aabb', 'alice'],
      isMember: true,
    ),
  ];

  testWidgets('shows grouped channel list when data loads', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('general'), findsOneWidget);
    expect(find.text('design-forum'), findsNothing);
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('Channels'), findsOneWidget);
    expect(find.text('FORUMS'), findsNothing);
    expect(find.text('DMs'), findsOneWidget);
    expect(find.text('Workspace'), findsOneWidget);
    expect(find.byTooltip('Create or start conversation'), findsOneWidget);
  });

  testWidgets('hides unjoined and archived channels from the main list', (
    tester,
  ) async {
    final channels = [
      ...testChannels,
      Channel(
        id: '4',
        name: 'open-stream',
        channelType: 'stream',
        visibility: 'open',
        description: 'Available to join',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 8,
        isMember: false,
      ),
      Channel(
        id: '5',
        name: 'archived-stream',
        channelType: 'stream',
        visibility: 'open',
        description: 'Archived channel',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 4,
        isMember: true,
        archivedAt: DateTime(2025, 1, 2),
      ),
    ];

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(channels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    // Unjoined and archived channels should not appear in the main list.
    expect(find.text('general'), findsOneWidget);
    expect(find.text('open-stream'), findsNothing);
    expect(find.text('archived-stream'), findsNothing);
  });

  testWidgets('shows empty state when no channels', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [channelsProvider.overrideWith(() => _FakeNotifier([]))],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('No conversations yet'), findsOneWidget);
  });

  testWidgets('shows error view with retry button', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [channelsProvider.overrideWith(() => _ErrorNotifier())],
      ),
    );
    // The error view is gated on a grace timer in ChannelsPage to absorb
    // transient AsyncError frames during relay reconnect. Pump once to mount
    // and schedule the timer, advance the fake clock past the grace window,
    // then pump again to flush the setState the timer triggered.
    await tester.pump();
    await tester.pump(const Duration(seconds: 3));
    await tester.pump();

    expect(find.text('Could not load channels'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('renders and clears unread dot indicator', (tester) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          20 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {'1': 10},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(
            () => _FakeNotifier(
              channels,
              observedEventsByChannel: {
                '1': [_observed(id: 'msg-1', createdAt: 20)],
              },
            ),
          ),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('channel-unread-dot-1')), findsOneWidget);

    readState.markContextRead('1', 20);
    await tester.pump();

    expect(find.byKey(const Key('channel-unread-dot-1')), findsNothing);
  });

  testWidgets('renders numeric unread count for counted events', (
    tester,
  ) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          30 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {'1': 10},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(
            () => _FakeNotifier(
              channels,
              observedEventsByChannel: {
                '1': [
                  _observed(
                    id: 'reply-1',
                    createdAt: 20,
                    rootId: 'root',
                    isThreadedReply: true,
                  ),
                  _observed(
                    id: 'reply-2',
                    createdAt: 30,
                    rootId: 'root',
                    isThreadedReply: true,
                  ),
                ],
              },
            ),
          ),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('channel-unread-1')), findsOneWidget);
    expect(find.text('2'), findsOneWidget);
    expect(find.byKey(const Key('channel-unread-dot-1')), findsNothing);
  });

  testWidgets('seeds first loaded channels as read', (tester) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          20 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(channels)),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(readState.seededContexts, {'1': 20});
    expect(readState.markedContexts, isEmpty);
    expect(find.byKey(const Key('channel-unread-1')), findsNothing);
  });

  testWidgets('waits for read-state readiness before initial seeding', (
    tester,
  ) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          20 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: false,
        pubkey: 'pk',
        contexts: {},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(channels)),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(readState.seededContexts, isEmpty);
    expect(readState.markedContexts, isEmpty);

    readState.setReady();
    await tester.pumpAndSettle();

    expect(readState.seededContexts, {'1': 20});
    expect(readState.markedContexts, isEmpty);
  });
}

class _FakeNotifier extends ChannelsNotifier {
  final List<Channel> _channels;
  final Map<String, Map<String, ObservedUnreadEvent>> _observedEventsByChannel;

  _FakeNotifier(
    this._channels, {
    Map<String, List<ObservedUnreadEvent>> observedEventsByChannel = const {},
  }) : _observedEventsByChannel = {
         for (final entry in observedEventsByChannel.entries)
           entry.key: {for (final event in entry.value) event.id: event},
       };

  @override
  Future<List<Channel>> build() async => _channels;

  @override
  Map<String, int> get latestObservedByChannel => {
    for (final entry in _observedEventsByChannel.entries)
      if (entry.value.isNotEmpty)
        entry.key: entry.value.values
            .map((event) => event.createdAt)
            .reduce((left, right) => left > right ? left : right),
  };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => _observedEventsByChannel;
}

class _ErrorNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() => Future.error('Connection refused');
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _FakePresenceNotifier extends PresenceNotifier {
  @override
  Future<String> build() async => 'online';
}

class _FakeReadStateNotifier extends ReadStateNotifier {
  final ReadStateState _initialState;
  final Map<String, int> seededContexts = {};
  final Map<String, int> markedContexts = {};

  _FakeReadStateNotifier(this._initialState);

  @override
  ReadStateState build() => _initialState;

  void setReady() {
    state = ReadStateState(
      isReady: true,
      pubkey: state.pubkey,
      contexts: state.contexts,
      version: state.version + 1,
      locallyForcedChannelIds: state.locallyForcedChannelIds,
    );
  }

  @override
  void seedContextRead(String contextId, int unixTimestamp) {
    seededContexts[contextId] = unixTimestamp;
    state = state.copyWithContext(contextId, unixTimestamp);
  }

  @override
  void markContextRead(String contextId, int unixTimestamp) {
    markedContexts[contextId] = unixTimestamp;
    state = state.copyWithContext(contextId, unixTimestamp);
  }
}

ObservedUnreadEvent _observed({
  required String id,
  required int createdAt,
  String? rootId,
  bool highPriority = false,
  bool isThreadedReply = false,
}) => makeObservedUnreadEvent(
  id: id,
  createdAt: createdAt,
  rootId: rootId,
  highPriority: highPriority,
  channelType: 'stream',
  isThreadedReply: isThreadedReply,
);
