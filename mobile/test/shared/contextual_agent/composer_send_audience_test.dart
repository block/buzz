import 'package:buzz/shared/contextual_agent/composer_send_audience.dart';
import 'package:buzz/shared/contextual_agent/contextual_agent_conversation_policy.dart';
import 'package:buzz/shared/contextual_agent/unaddressed_channel_agent_mode.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const human =
      '1111111111111111111111111111111111111111111111111111111111111111';
  const agentA =
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  const agentB =
      'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

  test('all-channel-agents merges verified agents', () {
    final result = resolveComposerSendAudience(
      conversation: 'channel',
      messagePosition: 'top-level',
      unaddressedMode: UnaddressedChannelAgentMode.allChannelAgents,
      keepAddressedAgentsActive: false,
      explicitMentionPubkeys: const [],
      explicitAgentPubkeys: const [],
      currentAgentPubkey: null,
      channelMemberPubkeys: const [human, agentA, agentB],
      verifiedChannelAgentPubkeys: const [agentA, agentB],
      persistentThreadAudience: const [],
    );
    expect(result.agentAudiencePubkeys.toSet(), {agentA, agentB});
    expect(result.retainDraft, isFalse);
  });

  test('mentions-only leaves agents empty without explicit', () {
    final result = resolveComposerSendAudience(
      conversation: 'channel',
      messagePosition: 'top-level',
      unaddressedMode: UnaddressedChannelAgentMode.mentionsOnly,
      keepAddressedAgentsActive: false,
      explicitMentionPubkeys: const [human],
      explicitAgentPubkeys: const [],
      currentAgentPubkey: null,
      channelMemberPubkeys: const [human, agentA],
      verifiedChannelAgentPubkeys: const [agentA],
      persistentThreadAudience: const [],
    );
    expect(result.agentAudiencePubkeys, isEmpty);
    expect(result.mentionPubkeys, [human]);
  });

  test('storage parse round-trip', () {
    expect(
      parseUnaddressedChannelAgentMode(null),
      UnaddressedChannelAgentMode.allChannelAgents,
    );
    expect(
      parseUnaddressedChannelAgentMode('mentions-only'),
      UnaddressedChannelAgentMode.mentionsOnly,
    );
    expect(
      unaddressedChannelAgentModeToStorage(
        UnaddressedChannelAgentMode.mentionsOnly,
      ),
      'mentions-only',
    );
  });
}
