import 'package:buzz/features/channels/agent_activity/active_agent_turns.dart';
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _channelId = 'channel-a';

void main() {
  test(
    'combines observer turns with bot-typing fallback for one channel',
    () async {
      final now = DateTime.utc(2026, 7, 30);
      final container = ProviderContainer(
        overrides: [
          activeAgentTurnsProvider.overrideWithValue([
            ActiveAgentTurn(
              agentPubkey: 'observer-bot',
              channelId: _channelId,
              turnId: 'turn-a',
              startedAt: now,
              lastActivityAt: now,
            ),
            ActiveAgentTurn(
              agentPubkey: 'other-channel-bot',
              channelId: 'channel-b',
              turnId: 'turn-b',
              startedAt: now,
              lastActivityAt: now,
            ),
            ActiveAgentTurn(
              agentPubkey: 'human',
              channelId: _channelId,
              turnId: 'turn-c',
              startedAt: now,
              lastActivityAt: now,
            ),
          ]),
          channelTypingProvider(_channelId).overrideWith(
            () => _FakeTypingNotifier([
              const TypingEntry(pubkey: 'typing-bot', expiresAtMs: 1),
              const TypingEntry(
                pubkey: 'thread-bot',
                threadHeadId: 'thread-1',
                expiresAtMs: 1,
              ),
              const TypingEntry(pubkey: 'human', expiresAtMs: 1),
            ]),
          ),
          channelMembersProvider(_channelId).overrideWith(
            (ref) async => [
              _member('observer-bot', 'bot'),
              _member('typing-bot', 'bot'),
              _member('thread-bot', 'bot'),
              _member('other-channel-bot', 'bot'),
              _member('human', 'member'),
            ],
          ),
        ],
      );
      addTearDown(container.dispose);

      await container.read(channelMembersProvider(_channelId).future);

      expect(container.read(workingBotPubkeysProvider(_channelId)), {
        'observer-bot',
        'typing-bot',
        'thread-bot',
      });
      expect(container.read(channelWorkingBotPubkeysProvider(_channelId)), {
        'observer-bot',
        'typing-bot',
      });
    },
  );
}

ChannelMember _member(String pubkey, String role) =>
    ChannelMember(pubkey: pubkey, role: role, joinedAt: DateTime.utc(2026));

class _FakeTypingNotifier extends ChannelTypingNotifier {
  _FakeTypingNotifier(this.entries) : super(_channelId);

  final List<TypingEntry> entries;

  @override
  List<TypingEntry> build() => entries;
}
