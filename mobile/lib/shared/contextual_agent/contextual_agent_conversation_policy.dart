// Contextual agent audience + reply-placement policy (Flutter).
//
// Contract: tests/fixtures/contextual-agent-conversation-cases.json
// Pure resolver plus send-path helpers for unaddressed-channel mode.

enum UnaddressedChannelAgentMode { allChannelAgents, mentionsOnly }

sealed class ReplyPlacement {
  const ReplyPlacement();
}

class TopLevelPlacement extends ReplyPlacement {
  const TopLevelPlacement();
}

class ThreadRootPlacement extends ReplyPlacement {
  const ThreadRootPlacement(this.eventId);
  final String eventId;
}

class UnconstrainedPlacement extends ReplyPlacement {
  const UnconstrainedPlacement();
}

class ContextualAgentConversationInput {
  const ContextualAgentConversationInput({
    required this.conversation,
    required this.messagePosition,
    required this.senderClass,
    required this.unaddressedMode,
    required this.keepAddressedAgentsActive,
    required this.explicitMentionPubkeys,
    required this.currentAgentPubkey,
    required this.channelMemberPubkeys,
    required this.verifiedChannelAgentPubkeys,
    this.unverifiedAgentPubkeys = const [],
    this.nonMemberAgentPubkeys = const [],
    required this.threadRootEventId,
    this.replyingUnderEventId,
    required this.persistentThreadAudience,
    required this.manualRemovedPubkeys,
    required this.recipientLoadError,
    this.humanMessageEventId,
  });

  final String conversation;
  final String messagePosition;
  final String senderClass;
  final UnaddressedChannelAgentMode unaddressedMode;
  final bool keepAddressedAgentsActive;
  final List<String> explicitMentionPubkeys;
  final String? currentAgentPubkey;
  final List<String> channelMemberPubkeys;
  final List<String> verifiedChannelAgentPubkeys;
  final List<String> unverifiedAgentPubkeys;
  final List<String> nonMemberAgentPubkeys;
  final String? threadRootEventId;
  final String? replyingUnderEventId;
  final List<String> persistentThreadAudience;
  final List<String> manualRemovedPubkeys;
  final bool recipientLoadError;
  final String? humanMessageEventId;
}

class ContextualAgentConversationDecision {
  const ContextualAgentConversationDecision({
    required this.audiencePubkeys,
    required this.replyPlacement,
    required this.sharedThread,
    required this.retainDraft,
    this.nestUnderAgentReply = false,
  });

  final List<String> audiencePubkeys;
  final ReplyPlacement replyPlacement;
  final bool sharedThread;
  final bool retainDraft;
  final bool nestUnderAgentReply;
}

String _normalizePubkey(String pubkey) => pubkey.trim().toLowerCase();

List<String> _uniqueSorted(Iterable<String> pubkeys) {
  final set = <String>{};
  for (final pk in pubkeys) {
    final n = _normalizePubkey(pk);
    if (n.isNotEmpty) set.add(n);
  }
  final list = set.toList()..sort();
  return list;
}

Set<String> _eligibleChannelAgents(ContextualAgentConversationInput input) {
  final members = input.channelMemberPubkeys.map(_normalizePubkey).toSet();
  return input.verifiedChannelAgentPubkeys
      .map(_normalizePubkey)
      .where(members.contains)
      .toSet();
}

List<String> _filterToEligible(List<String> candidates, Set<String> eligible) {
  return _uniqueSorted(
    candidates.map(_normalizePubkey).where(eligible.contains),
  );
}

ReplyPlacement _placementFor(
  ContextualAgentConversationInput input,
  int audienceCount,
) {
  if (input.messagePosition == 'in-thread' &&
      input.threadRootEventId != null &&
      input.threadRootEventId!.isNotEmpty) {
    return ThreadRootPlacement(input.threadRootEventId!);
  }
  if (audienceCount >= 2) {
    final eventId = input.humanMessageEventId ?? input.threadRootEventId;
    if (eventId != null && eventId.isNotEmpty) {
      return ThreadRootPlacement(eventId);
    }
  }
  return const TopLevelPlacement();
}

/// Resolve audience and reply placement for a human/agent send path.
ContextualAgentConversationDecision resolveContextualAgentConversation(
  ContextualAgentConversationInput input,
) {
  if (input.recipientLoadError) {
    return const ContextualAgentConversationDecision(
      audiencePubkeys: [],
      replyPlacement: TopLevelPlacement(),
      sharedThread: false,
      retainDraft: true,
    );
  }

  if (input.senderClass == 'agent') {
    return const ContextualAgentConversationDecision(
      audiencePubkeys: [],
      replyPlacement: UnconstrainedPlacement(),
      sharedThread: false,
      retainDraft: false,
    );
  }

  if (input.conversation == 'direct') {
    final current = input.currentAgentPubkey;
    final audience = current == null || current.isEmpty
        ? <String>[]
        : [_normalizePubkey(current)];
    return ContextualAgentConversationDecision(
      audiencePubkeys: audience,
      replyPlacement: _placementFor(input, audience.length),
      sharedThread: false,
      retainDraft: false,
    );
  }

  final eligible = _eligibleChannelAgents(input);
  final removed = input.manualRemovedPubkeys.map(_normalizePubkey).toSet();

  final explicit = _filterToEligible(
    input.explicitMentionPubkeys,
    eligible,
  ).where((pk) => !removed.contains(pk)).toList();

  late final List<String> audience;
  if (explicit.isNotEmpty) {
    audience = explicit;
  } else {
    final persistent = input.keepAddressedAgentsActive
        ? _filterToEligible(
            input.persistentThreadAudience,
            eligible,
          ).where((pk) => !removed.contains(pk)).toList()
        : <String>[];
    if (persistent.isNotEmpty) {
      audience = persistent;
    } else if (input.unaddressedMode ==
        UnaddressedChannelAgentMode.allChannelAgents) {
      audience = _uniqueSorted(
        eligible,
      ).where((pk) => !removed.contains(pk)).toList();
    } else {
      audience = [];
    }
  }

  return ContextualAgentConversationDecision(
    audiencePubkeys: audience,
    replyPlacement: _placementFor(input, audience.length),
    sharedThread: audience.length >= 2,
    retainDraft: false,
  );
}
