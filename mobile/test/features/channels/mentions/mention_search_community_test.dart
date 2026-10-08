import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

// The directory search must belong to one community. Two communities can
// share a signing key and the session notifier, so only the relay config
// tells them apart. A search that community A started must not show its
// people, or its error, in community B.

final _me = 'a' * 64;
final _alice = 'b' * 64;
final _bob = 'c' * 64;

const _channelId = 'stream-1';
const MentionChooserArgs _args = (
  channelId: _channelId,
  query: 'al',
  opening: 1,
);

final _stream = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: _me,
  createdAt: DateTime.utc(2026),
  memberCount: 1,
  isMember: true,
);

NostrEvent _profile(String pubkey, String name) => NostrEvent(
  id: '$pubkey-profile',
  pubkey: pubkey,
  createdAt: 1700000000,
  kind: 0,
  tags: const [],
  content: '{"display_name":"$name"}',
  sig: 'sig',
);

/// Holds every search request open until the test answers it.
class _HeldSearchSession extends RelaySessionNotifier {
  final searches = <Completer<List<NostrEvent>>>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final search = Completer<List<NostrEvent>>();
    searches.add(search);
    return search.future;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];
}

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [_stream];

  @override
  List<ChannelMember> cachedMembersForChannel(String channelId) => const [];
}

class _CommunityA extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'http://community-a.test');
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late _HeldSearchSession session;
  late ProviderContainer container;
  var memberChoices = <ChannelMember>[];

  setUp(() async {
    session = _HeldSearchSession();
    memberChoices = [];
    container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        relayConfigProvider.overrideWith(_CommunityA.new),
        currentPubkeyProvider.overrideWith((ref) => _me),
        channelsProvider.overrideWith(_Channels.new),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => memberChoices),
        agentDirectoryProvider.overrideWith((ref) async => const []),
        agentOwnersProvider.overrideWith((ref) async => const {}),
      ],
    );
    addTearDown(container.dispose);
    // Keep the chooser open, the way a typed `@al` does.
    final candidates = container.listen(
      mentionCandidatesProvider(_args),
      (_, _) {},
    );
    final failed = container.listen(
      mentionSearchFailedProvider(_args),
      (_, _) {},
    );
    addTearDown(candidates.close);
    addTearDown(failed.close);
    await container.read(channelsProvider.future);
  });

  Future<void> passTypingPause() async {
    await Future<void>.delayed(mentionSearchDebounce * 2);
    await container.pump();
  }

  Future<void> switchToCommunityB() async {
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'http://community-b.test', nsec: null);
    await container.pump();
  }

  List<String> shownPubkeys() => [
    for (final candidate in container.read(mentionCandidatesProvider(_args)))
      candidate.pubkey,
  ];

  test('production mention chooser distinguishes same-name agents', () async {
    memberChoices = [
      for (final key in [_alice, _bob])
        ChannelMember(
          pubkey: key,
          displayName: 'Albert',
          role: 'bot',
          joinedAt: DateTime(2026),
        ),
    ];
    container.invalidate(channelMembersProvider(_channelId));
    await container.read(channelMembersProvider(_channelId).future);
    await container.pump();
    final choices = container.read(mentionCandidatesProvider(_args));
    expect(choices.map((choice) => choice.pubkey).toSet(), {_alice, _bob});
    expect(choices.map((choice) => choice.pickerLabel).toSet(), hasLength(2));
    for (final choice in choices) {
      expect(choice.pickerLabel, startsWith('Albert · '));
    }
  });

  test('a late search result from community A is not shown in B', () async {
    await passTypingPause();
    expect(session.searches, hasLength(1));

    await switchToCommunityB();
    session.searches.first.complete([_profile(_alice, 'Alice')]);
    await container.pump();
    await passTypingPause();

    expect(shownPubkeys(), isNot(contains(_alice)));

    expect(session.searches, hasLength(2));
    session.searches.last.complete([_profile(_bob, 'Albert')]);
    await container.pump();
    expect(shownPubkeys(), [_bob]);
  });

  test('a late search error from community A is not shown in B', () async {
    await passTypingPause();
    expect(session.searches, hasLength(1));

    await switchToCommunityB();
    session.searches.first.completeError(StateError('community A failed'));
    await container.pump();
    await passTypingPause();

    expect(container.read(mentionSearchFailedProvider(_args)), isFalse);

    expect(session.searches, hasLength(2));
    session.searches.last.complete([_profile(_bob, 'Albert')]);
    await container.pump();
    expect(container.read(mentionSearchFailedProvider(_args)), isFalse);
    expect(shownPubkeys(), [_bob]);
  });

  test('people found in community A are not kept while B searches', () async {
    await passTypingPause();
    session.searches.first.complete([_profile(_alice, 'Alice')]);
    await container.pump();
    expect(shownPubkeys(), [_alice]);

    await switchToCommunityB();
    await passTypingPause();

    expect(session.searches, hasLength(2));
    expect(shownPubkeys(), isNot(contains(_alice)));
  });
}
