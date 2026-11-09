import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

final _human = 'a' * 64;
final _agent = 'b' * 64;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('addMembers records agents with the bot role', () async {
    final keys = nostr.Keys.generate();
    final session = _RecordingSession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final actions = container.read(
      Provider(
        (ref) => ChannelActions(
          ref: ref,
          session: session,
          signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
          currentPubkey: keys.public,
        ),
      ),
    );

    await actions.addMembers(
      channelId: 'channel',
      pubkeys: [_human, _agent.toUpperCase()],
      agentPubkeys: {_agent},
    );

    final roles = {
      for (final event in session.published.where((e) => e.kind == 9000))
        event.tags.firstWhere((tag) => tag.first == 'p')[1]: event.tags
            .firstWhere((tag) => tag.first == 'role')[1],
    };
    expect(roles, {_human: 'member', _agent: 'bot'});
  });
}

class _RecordingSession extends RelaySessionNotifier {
  final published = <NostrEvent>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return event;
  }
}
