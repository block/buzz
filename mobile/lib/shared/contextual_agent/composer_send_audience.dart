import 'contextual_agent_conversation_policy.dart';

/// Merge explicit mentions with implicit contextual-agent audience (Desktop parity).
class ComposerSendAudienceResult {
  const ComposerSendAudienceResult({
    required this.mentionPubkeys,
    required this.agentAudiencePubkeys,
    required this.retainDraft,
  });

  final List<String> mentionPubkeys;
  final List<String> agentAudiencePubkeys;
  final bool retainDraft;
}

List<String> _uniqueNormalized(Iterable<String> pubkeys) {
  final set = <String>{};
  for (final pk in pubkeys) {
    final n = pk.trim().toLowerCase();
    if (n.isNotEmpty) set.add(n);
  }
  return set.toList();
}

ComposerSendAudienceResult resolveComposerSendAudience({
  required String conversation,
  required String messagePosition,
  required UnaddressedChannelAgentMode unaddressedMode,
  required bool keepAddressedAgentsActive,
  required List<String> explicitMentionPubkeys,
  required List<String> explicitAgentPubkeys,
  required String? currentAgentPubkey,
  required List<String> channelMemberPubkeys,
  required List<String> verifiedChannelAgentPubkeys,
  required List<String> persistentThreadAudience,
  List<String> manualRemovedPubkeys = const [],
  String? threadRootEventId,
  bool recipientLoadError = false,
}) {
  final explicitAgentSet = _uniqueNormalized(explicitAgentPubkeys).toSet();
  final decision = resolveContextualAgentConversation(
    ContextualAgentConversationInput(
      conversation: conversation,
      messagePosition: messagePosition,
      senderClass: 'human',
      unaddressedMode: unaddressedMode,
      keepAddressedAgentsActive: keepAddressedAgentsActive,
      explicitMentionPubkeys: explicitAgentSet.toList(),
      currentAgentPubkey: currentAgentPubkey,
      channelMemberPubkeys: channelMemberPubkeys,
      verifiedChannelAgentPubkeys: verifiedChannelAgentPubkeys,
      threadRootEventId: threadRootEventId,
      persistentThreadAudience: persistentThreadAudience,
      manualRemovedPubkeys: manualRemovedPubkeys,
      recipientLoadError: recipientLoadError,
    ),
  );

  // Always retain authored agent @mentions (DM expansion to a new agent)
  // while still applying implicit/persistent audience from policy.
  final agentAudience = _uniqueNormalized([
    ...decision.audiencePubkeys,
    ...explicitAgentSet,
  ]);
  final humanMentions = _uniqueNormalized(
    explicitMentionPubkeys,
  ).where((pk) => !explicitAgentSet.contains(pk)).toList();

  return ComposerSendAudienceResult(
    mentionPubkeys: _uniqueNormalized([...humanMentions, ...agentAudience]),
    agentAudiencePubkeys: agentAudience,
    retainDraft: decision.retainDraft,
  );
}
