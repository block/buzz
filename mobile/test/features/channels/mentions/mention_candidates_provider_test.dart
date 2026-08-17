import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('mention user search omits archived profiles but keeps self', () async {
    const archivedKey =
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const canonicalKey =
        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const selfKey =
        'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(
          () => _MentionSearchFakeRelaySession([
            _profile(archivedKey, 'OPUS'),
            _profile(canonicalKey, 'OPUS'),
            _profile(selfKey, 'Mr. Wayne'),
          ]),
        ),
        myPubkeyProvider.overrideWithValue(selfKey),
        relayArchivedIdentityPubkeysProvider.overrideWith(
          (ref) async => const {archivedKey, selfKey},
        ),
      ],
    );
    addTearDown(container.dispose);

    final provider = mentionUserSearchProvider('opus');
    final subscription = container.listen(provider, (_, _) {});
    addTearDown(subscription.close);
    final results = await container.read(provider.future);

    expect(results.map((profile) => profile.pubkey), [canonicalKey, selfKey]);
  });
}

NostrEvent _profile(String pubkey, String name) => NostrEvent(
  id: '$pubkey-profile',
  pubkey: pubkey,
  createdAt: 1700000000,
  kind: 0,
  tags: const [],
  content: '{"display_name":"$name"}',
  sig: 'sig',
);

class _MentionSearchFakeRelaySession extends RelaySessionNotifier {
  _MentionSearchFakeRelaySession(this.profileEvents);

  final List<NostrEvent> profileEvents;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => profileEvents;
}
