import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/channels/mentions/mention_ranking.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

// Before a name is typed, `@` lists the channel's members. A recipient
// picker has no channel members to list, so it lists the named people the
// viewer shares an open channel or DM with.

final _me = 'f' * 64;
final _ada = 'a' * 64;
final _nameless = 'b' * 64;
final _carol = 'c' * 64;
final _dan = 'd' * 64;
final _eve = 'e' * 64;
final _ghost = '9' * 64;
final _zed = '8' * 64;

/// An agent shared with the viewer in `general`, sorting after every person.
final _zedAgent = AgentDirectoryEntry(
  pubkey: _zed,
  displayName: 'Zed',
  respondTo: 'anyone',
  channelIds: const ['general'],
);

/// Profile requests per person.
final _requests = <String, int>{};

Channel _channel(
  String id, {
  String type = 'stream',
  bool member = true,
  DateTime? archivedAt,
  List<String> participants = const [],
}) => Channel(
  id: id,
  name: id,
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: _me,
  createdAt: DateTime.utc(2026),
  memberCount: 2,
  isMember: member,
  archivedAt: archivedAt,
  participantPubkeys: participants,
);

ChannelMember _member(String pubkey) =>
    ChannelMember(pubkey: pubkey, role: 'member', joinedAt: DateTime.utc(2026));

class _Session extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> searchProfiles(
    String query, {
    int limit = 50,
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(0)) {
      for (final author in filter.authors ?? const <String>[]) {
        _profileReads[author] = (_profileReads[author] ?? 0) + 1;
      }
    }
    return const [];
  }
}

/// Relay profile reads per person, through the real user cache.
final _profileReads = <String, int>{};

/// Named people in the `crowd` channel; none unless a test sets it.
var _crowd = <String>[];

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    _channel('general'),
    _channel('crowd'),
    _channel('old', archivedAt: DateTime.utc(2026)),
    _channel('elsewhere', member: false),
    _channel('dm', type: 'dm', participants: [_me, _eve]),
  ];

  @override
  List<ChannelMember> cachedMembersForChannel(String channelId) =>
      switch (channelId) {
        'general' => [
          _member(_me),
          _member(_ada),
          _member(_nameless),
          _member(_ghost),
        ],
        'old' => [_member(_carol)],
        'elsewhere' => [_member(_dan)],
        'crowd' => [for (final pubkey in _crowd) _member(pubkey)],
        _ => const [],
      };

  /// Republishes the same list, as a new message or presence change does.
  void touch() => state = AsyncData([...state.requireValue]);
}

class _Cache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {
    for (final (pubkey, name) in [
      (_me, 'Me'),
      (_ada, 'Ada'),
      (_carol, 'Carol'),
      (_dan, 'Dan'),
      (_eve, 'Eve'),
    ])
      pubkey: UserProfile(pubkey: pubkey, displayName: name),
    for (final (i, pubkey) in _crowd.indexed)
      pubkey: UserProfile(
        pubkey: pubkey,
        displayName: 'Crowd ${i.toString().padLeft(2, '0')}',
      ),
  };

  /// Loads a name for an unnamed person when the picker asks for it.
  @override
  Future<bool> preload(List<String> pubkeys) async {
    for (final pubkey in pubkeys) {
      _requests[pubkey] = (_requests[pubkey] ?? 0) + 1;
    }
    if (pubkeys.contains(_nameless)) {
      state = {
        ...state,
        _nameless: UserProfile(pubkey: _nameless, displayName: 'Bo'),
      };
    }
    return true;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  ProviderContainer container0({
    List<AgentDirectoryEntry> agents = const [],
    bool agentsLoading = false,
    bool realCache = false,
  }) => ProviderContainer(
    retry: (_, _) => null,
    overrides: [
      relaySessionProvider.overrideWith(_Session.new),
      currentPubkeyProvider.overrideWith((ref) => _me),
      channelsProvider.overrideWith(_Channels.new),
      if (!realCache) userCacheProvider.overrideWith(_Cache.new),
      agentDirectoryProvider.overrideWith(
        (ref) => agentsLoading
            ? Completer<List<AgentDirectoryEntry>>().future
            : Future.value(agents),
      ),
      agentOwnersProvider.overrideWith((ref) async => const {}),
    ],
  );

  Future<List<String>> pick(
    String query, {
    String? channelId,
    List<AgentDirectoryEntry> agents = const [],
    bool agentsLoading = false,
  }) async {
    _requests.clear();
    final container = container0(agents: agents, agentsLoading: agentsLoading);
    addTearDown(container.dispose);
    final search = (
      query: query,
      historyKey: addMembersMentionHistoryKey,
      unsharedAgents: true,
      channelId: channelId,
    );
    final sub = container.listen(
      recipientCandidatesProvider(search),
      (_, _) {},
    );
    addTearDown(sub.close);
    await container.read(channelsProvider.future);
    await Future<void>.delayed(mentionSearchDebounce * 2);
    await container.pump();
    return [
      for (final candidate in container.read(
        recipientCandidatesProvider(search),
      ))
        candidate.label,
    ];
  }

  test(
    'an empty query lists people from open channels and DMs, loading names',
    () async {
      expect(await pick(''), ['Ada', 'Bo', 'Eve']);
      // Someone with no profile is asked for once, not on each rebuild.
      expect(_requests[_ghost], 1);
    },
  );

  test('agents keep their rows ahead of known people', () async {
    expect(await pick('', agents: [_zedAgent]), ['Zed', 'Ada', 'Bo', 'Eve']);
    // Known people do not wait for an agent list that has not loaded, as
    // the viewer's own agents never do offline.
    expect(await pick('', agentsLoading: true), ['Ada', 'Bo', 'Eve']);
  });

  test('the real cache reads each unnamed person once per opening', () async {
    _profileReads.clear();
    final container = container0(realCache: true);
    addTearDown(container.dispose);
    final search = (
      query: '',
      historyKey: addMembersMentionHistoryKey,
      unsharedAgents: true,
      channelId: null,
    );
    final sub = container.listen(
      recipientCandidatesProvider(search),
      (_, _) {},
    );
    addTearDown(sub.close);
    await container.read(channelsProvider.future);
    for (var i = 0; i < 3; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
      await container.pump();
      (container.read(channelsProvider.notifier) as _Channels).touch();
    }
    await Future<void>.delayed(const Duration(milliseconds: 100));
    expect(_profileReads.keys, containsAll([_ada, _nameless, _ghost, _eve]));
    expect(_profileReads.values.toSet(), {1});
  });

  test('Add members skips the channel\'s own members', () async {
    expect(await pick('', channelId: 'general'), ['Eve']);
  });

  test('at most one picker list of people is gathered', () async {
    _crowd = [
      for (var i = 0; i < 80; i++) i.toRadixString(16).padLeft(64, '1'),
    ];
    addTearDown(() => _crowd = []);
    final people = await pick('');
    // One gathered slot is the person with no profile.
    expect(people, hasLength(mentionSuggestionLimit - 1));
    expect(people, containsAll(['Ada', 'Bo', 'Eve']));
  });

  test('a typed query finds people by the directory search', () async {
    expect(await pick('Ad'), isEmpty);
  });
}
