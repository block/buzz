import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_directory.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

// Headers show a NIP-FI verified name in place of the self-declared one, so
// the mention picker must find, rank and insert people by that same name. An
// impostor who copies a verified name into kind 0 must not be the only match.

final _me = 'f' * 64;
final _ada = 'a' * 64;
final _impostor = 'b' * 64;
const _channelId = 'stream-1';

final _stream = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: _me,
  createdAt: DateTime.utc(2026),
  memberCount: 3,
  isMember: true,
);

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
  }) async => const [];
}

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [_stream];

  @override
  List<ChannelMember> cachedMembersForChannel(String channelId) => const [];
}

class _Verified extends VerifiedProfilesNotifier {
  @override
  Map<String, VerifiedProfile> build() => {
    _ada: const VerifiedProfile(
      displayName: 'Ada Lovelace',
      username: 'ada',
      issuerLabel: 'Example Corp',
      verifiedAt: 1000,
      checked: true,
      multipleIssuers: false,
    ),
  };
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<List<String>> pick(String query) async {
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(_Session.new),
        currentPubkeyProvider.overrideWith((ref) => _me),
        channelsProvider.overrideWith(_Channels.new),
        channelMembersProvider(_channelId).overrideWith(
          (ref) async => [
            ChannelMember(
              pubkey: _ada,
              role: 'member',
              joinedAt: DateTime.utc(2026),
              displayName: 'ada_l',
            ),
            ChannelMember(
              pubkey: _impostor,
              role: 'member',
              joinedAt: DateTime.utc(2026),
              displayName: 'Ada Lovelace',
            ),
          ],
        ),
        agentDirectoryProvider.overrideWith((ref) async => const []),
        agentOwnersProvider.overrideWith((ref) async => const {}),
        verifiedProfilesProvider.overrideWith(_Verified.new),
      ],
    );
    addTearDown(container.dispose);
    final args = (channelId: _channelId, query: query, opening: 1);
    final sub = container.listen(mentionCandidatesProvider(args), (_, _) {});
    addTearDown(sub.close);
    await container.read(channelsProvider.future);
    await container.read(channelMembersProvider(_channelId).future);
    await Future<void>.delayed(mentionSearchDebounce * 2);
    await container.pump();
    return [
      for (final candidate in container.read(mentionCandidatesProvider(args)))
        '${candidate.pubkey == _ada ? 'ada' : 'impostor'}=${candidate.label}',
    ];
  }

  test('typing a verified name finds the verified person', () async {
    expect(await pick('Ada Lovelace'), contains('ada=Ada Lovelace'));
  });

  test('a verified person is inserted by the verified name', () async {
    final rows = await pick('Ada');
    expect(rows, contains('ada=Ada Lovelace'));
    expect(rows, isNot(contains('ada=ada_l')));
  });

  test('the self-declared name still finds the verified person', () async {
    expect(await pick('ada_l'), ['ada=Ada Lovelace']);
  });
}
