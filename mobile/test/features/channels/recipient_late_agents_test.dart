import 'dart:async';

import 'package:buzz/features/channels/add_members_sheet.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/recipient_directory_session.dart';

// The viewer's own agents can load after the known people an empty query
// lists, as on first load or a reconnect. They still take their rows ahead
// of the people, and a reload that brings no new agent moves nothing.

final _me = 'f' * 64;
final _people = [
  for (var i = 0; i < 60; i++) '$i'.padLeft(4, '0').padLeft(64, '2'),
];
final _ann = OwnedAgentDirectoryEntry(pubkey: 'a' * 64, displayName: 'Ann');
final _bea = OwnedAgentDirectoryEntry(pubkey: 'b' * 64, displayName: 'Bea');

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: 'crowd',
      name: 'crowd',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: _me,
      createdAt: DateTime.utc(2026),
      memberCount: _people.length,
      isMember: true,
    ),
  ];

  @override
  List<ChannelMember> cachedMembersForChannel(String channelId) => [
    if (channelId == 'crowd')
      for (final pubkey in _people)
        ChannelMember(
          pubkey: pubkey,
          role: 'member',
          joinedAt: DateTime.utc(2026),
        ),
  ];
}

class _Cache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {
    for (final (i, pubkey) in _people.indexed)
      pubkey: UserProfile(
        pubkey: pubkey,
        displayName: 'Person ${i.toString().padLeft(2, '0')}',
      ),
  };

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

/// An agent list. Each [reload] starts a load that keeps the earlier list,
/// as a session status change does, until [land].
class _Loads<T> extends Notifier<int> {
  final loads = [Completer<List<T>>()];

  @override
  int build() => 0;

  void reload() {
    loads.add(Completer());
    state++;
  }

  void land(List<T> agents) => loads.last.complete(agents);
}

/// The viewer's own agents.
final _owned = NotifierProvider<_Loads<OwnedAgentDirectoryEntry>, int>(
  _Loads.new,
);

/// Relay directory agents that answer anyone.
final _shared = NotifierProvider<_Loads<AgentDirectoryEntry>, int>(_Loads.new);

AgentDirectoryEntry _sharedAgent(String pubkey, String name) =>
    AgentDirectoryEntry(
      pubkey: pubkey,
      displayName: name,
      respondTo: 'anyone',
      channelIds: const ['crowd'],
    );

void main() {
  late ProviderContainer container;

  Future<void> open(
    WidgetTester tester, {
    bool loadFirst = false,
    bool sharedLater = false,
  }) async {
    container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => _me),
        channelsProvider.overrideWith(_Channels.new),
        userCacheProvider.overrideWith(_Cache.new),
        channelMembersProvider('channel').overrideWith((ref) async => []),
        agentDirectoryProvider.overrideWith((ref) {
          final load = ref.watch(_shared);
          return ref.read(_shared.notifier).loads[load].future;
        }),
        agentOwnersProvider.overrideWith((ref) async => const {}),
        ownedAgentDirectoryProvider.overrideWith((ref) {
          final load = ref.watch(_owned);
          return ref.read(_owned.notifier).loads[load].future;
        }),
        relaySessionProvider.overrideWith(
          () => RecipientDirectorySession({_people.first: 'Person 00'}),
        ),
      ],
    );
    if (!sharedLater) container.read(_shared.notifier).land(const []);
    if (loadFirst) {
      container.listen(ownedAgentDirectoryProvider, (_, _) {});
      container.read(_owned.notifier).land(const []);
      await container.read(ownedAgentDirectoryProvider.future);
      container.read(_owned.notifier).reload();
    }
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const Scaffold(
            body: AddChannelMembersSheet(
              channelId: 'channel',
              existingPubkeys: {},
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  _Loads<OwnedAgentDirectoryEntry> owned() => container.read(_owned.notifier);
  _Loads<AgentDirectoryEntry> shared() => container.read(_shared.notifier);

  /// Closes the sheet and lets its pending timers run.
  Future<void> close(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
    await tester.pump(const Duration(seconds: 30));
  }

  /// The labels of the rows built, top first.
  List<String> rows(WidgetTester tester) => [
    for (final tile in tester.widgetList<ListTile>(
      find.descendant(
        of: find.byKey(const ValueKey('add-channel-members-results')),
        matching: find.byType(ListTile),
      ),
    ))
      (tile.title! as Text).data!,
  ];

  testWidgets('agents that load after known people show ahead of them', (
    tester,
  ) async {
    await open(tester);
    expect(rows(tester).first, 'Person 00');

    // First load, with 50 people already shown.
    owned().land([_ann]);
    await tester.pumpAndSettle();
    expect(rows(tester).take(2), ['Ann', 'Person 00']);

    // A flapping link that brings no new agent moves nothing.
    final shown = rows(tester);
    for (var i = 0; i < 2; i++) {
      owned().reload();
      await tester.pumpAndSettle();
      expect(rows(tester), shown);
      owned().land([_ann]);
      await tester.pumpAndSettle();
      expect(rows(tester), shown);
    }

    // A reload that brings a new agent gives it a row ahead of the people.
    owned().reload();
    await tester.pumpAndSettle();
    owned().land([_ann, _bea]);
    await tester.pumpAndSettle();
    expect(rows(tester).take(3), ['Ann', 'Bea', 'Person 00']);
    await close(tester);
  });

  testWidgets('a picker opened during a reload shows the agents it brings', (
    tester,
  ) async {
    await open(tester, loadFirst: true);
    expect(rows(tester).first, 'Person 00');
    owned().land([_ann]);
    await tester.pumpAndSettle();
    expect(rows(tester).take(2), ['Ann', 'Person 00']);
    await close(tester);
  });

  testWidgets('typed rows keep their places when an agent loads', (
    tester,
  ) async {
    await open(tester);
    await tester.enterText(
      find.byKey(const ValueKey('add-channel-members-search')),
      'Person',
    );
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    expect(rows(tester), ['Person 00']);
    owned().land([
      OwnedAgentDirectoryEntry(pubkey: 'c' * 64, displayName: 'Person 00 Bot'),
    ]);
    await tester.pumpAndSettle();
    expect(rows(tester), ['Person 00', 'Person 00 Bot']);
    await close(tester);
  });

  testWidgets('more agents than rows fill the list and stay put', (
    tester,
  ) async {
    final many = [
      for (var i = 0; i < 55; i++)
        OwnedAgentDirectoryEntry(
          pubkey: '$i'.padLeft(4, '0').padLeft(64, '3'),
          displayName: 'Agent ${i.toString().padLeft(2, '0')}',
        ),
    ];
    await open(tester);
    owned().land(many);
    await tester.pumpAndSettle();
    final shown = rows(tester);
    expect(shown.first, 'Agent 00');
    for (var i = 0; i < 2; i++) {
      owned().reload();
      await tester.pumpAndSettle();
      owned().land(many);
      await tester.pumpAndSettle();
      expect(rows(tester), shown);
    }
    // A reload that only reorders the agents moves nothing either.
    owned().reload();
    await tester.pumpAndSettle();
    owned().land([
      ...many.take(54),
      OwnedAgentDirectoryEntry(pubkey: many.last.pubkey, displayName: 'Aa'),
    ]);
    await tester.pumpAndSettle();
    expect(rows(tester), shown);
    await tester.drag(
      find.byKey(const ValueKey('add-channel-members-results')),
      const Offset(0, -10000),
    );
    await tester.pumpAndSettle();
    expect(rows(tester).last, 'Agent 49');
    await close(tester);
  });

  testWidgets('agents keep their rows while the other list reloads', (
    tester,
  ) async {
    final sam = _sharedAgent('5' * 64, 'Sam');
    final zed = _sharedAgent('6' * 64, 'Zed');
    final bea = _bea;
    for (final ownedFirst in [true, false]) {
      await open(tester, sharedLater: true);
      owned().land([_ann]);
      shared().land([sam]);
      await tester.pumpAndSettle();
      expect(rows(tester).take(3), ['Ann', 'Sam', 'Person 00']);

      // Both lists reload after a reconnect. The first to land brings a new
      // agent; the other's agent keeps its row until its list lands, and
      // that landing moves nothing.
      owned().reload();
      shared().reload();
      await tester.pumpAndSettle();
      if (ownedFirst) {
        owned().land([_ann, bea]);
      } else {
        shared().land([sam, zed]);
      }
      await tester.pumpAndSettle();
      final restarted = rows(tester);
      expect(restarted.take(4), [
        'Ann',
        'Sam',
        ownedFirst ? 'Bea' : 'Zed',
        'Person 00',
      ]);
      if (ownedFirst) {
        shared().land([sam]);
      } else {
        owned().land([_ann]);
      }
      await tester.pumpAndSettle();
      expect(rows(tester), restarted);
      await close(tester);
    }
  });
}
