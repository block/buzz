import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/mentions/agent_policy_test.dart' show PolicySession;

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

void main() {
  test(
    'production picker preserves policy loading/error instead of empty success',
    () async {
      final viewer = 'a' * 64;
      final agent = 'b' * 64;
      final human = 'c' * 64;
      for (final member in [false, true]) {
        final directory = Completer<List<AgentDirectoryEntry>>();
        final c = ProviderContainer(
          overrides: [
            relaySessionProvider.overrideWith(() => PolicySession([])),
            channelsProvider.overrideWith(_Channels.new),
            currentPubkeyProvider.overrideWithValue(viewer),
            agentDirectoryProvider.overrideWith((ref) => directory.future),
            agentOwnersProvider.overrideWith((ref) async => {agent: viewer}),
            channelMembersProvider('room').overrideWith(
              (ref) async => [
                ChannelMember(
                  pubkey: human,
                  role: 'member',
                  joinedAt: DateTime(2024),
                ),
                if (member)
                  ChannelMember(
                    pubkey: agent,
                    role: 'bot',
                    joinedAt: DateTime(2024),
                  ),
              ],
            ),
            mentionUserSearchProvider('').overrideWith(
              (ref) async => [
                if (!member) UserProfile(pubkey: agent, ownerPubkey: viewer),
              ],
            ),
          ],
        );
        final picker = mentionCandidatesProvider((
          channelId: 'room',
          query: '',
        ));
        final subscription = c.listen(picker, (_, _) {});
        await c.read(channelsProvider.future);
        await c.read(agentOwnersProvider.future);
        await c.read(channelMembersProvider('room').future);
        await c.read(mentionUserSearchProvider('').future);
        expect(c.read(picker).map((candidate) => candidate.pubkey), [human]);
        directory.completeError(StateError('policy unavailable'));
        await expectLater(
          c.read(agentDirectoryProvider.future),
          throwsStateError,
        );
        await c.pump();
        expect(c.read(picker).map((candidate) => candidate.pubkey), [human]);
        subscription.close();
        c.dispose();
      }
    },
  );
}
