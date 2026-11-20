part of '../compose_bar.dart';

/// The viewer's latest message among [messages], with the pubkeys it notified.
///
/// [messages] are what the conversation displays, oldest first: deleted
/// messages are already gone, and the channel and thread views break
/// same-second ties differently, so the last own message in that order is the
/// one the viewer sees as latest.
///
/// Only notifying `p` tags count: `mention` reference tags never notified
/// anyone, so they are not carried forward.
({String messageId, List<String> pubkeys})? latestOwnMessageMentions(
  List<TimelineMessage> messages, {
  required String? viewer,
}) {
  final me = viewer?.toLowerCase();
  if (me == null || me.isEmpty) return null;
  final latest = messages.reversed
      .where(
        (message) => !message.isSystem && message.pubkey.toLowerCase() == me,
      )
      .firstOrNull;
  if (latest == null) return null;
  final pubkeys = <String>[];
  for (final tag in latest.tags) {
    if (tag.length < 2 || tag[0] != 'p') continue;
    final pubkey = tag[1].toLowerCase();
    if (pubkey != me && !pubkeys.contains(pubkey)) pubkeys.add(pubkey);
  }
  return (messageId: latest.id, pubkeys: pubkeys);
}

/// Prefills an empty composer with the agents the viewer's latest message in
/// this conversation notified, matching desktop's "remember mentioned agents".
///
/// The prefill is not a draft: [prefillText] holds it so the draft saver skips
/// it until the viewer edits. Any edit ends the prefill for that message, so a
/// removed agent stays removed until the composer is shown again or the
/// viewer's latest message changes (as after a send).
void _useAgentMentionPrefill({
  required WidgetRef ref,
  required String channelId,
  required List<TimelineMessage>? scopeMessages,
  required String draftKey,
  required _MarkdownEditingController controller,
  required ObjectRef<String?> prefillText,
  required ObjectRef<bool> isModifyingText,
  required bool hasAttachments,
}) {
  final viewer = ref.watch(currentPubkeyProvider);
  // The composer rebuilds on every keystroke; rescan only when the loaded
  // messages or the viewer change.
  final source = useMemoized(
    () => scopeMessages == null
        ? null
        : latestOwnMessageMentions(scopeMessages, viewer: viewer),
    [scopeMessages, viewer],
  );
  // Current members only: an agent that has left is not carried forward.
  final members = source == null || source.pubkeys.isEmpty
      ? null
      : ref.watch(channelMembersProvider(channelId)).asData?.value;
  final memberCandidates = members == null
      ? const <MentionCandidate>[]
      : buildMentionCandidates(
          members: members,
          relayAgents: const [],
          sharedChannelIds: const {},
          userCache: ref.watch(userCacheProvider),
          ownerByAgentPubkey:
              ref.watch(agentOwnersProvider).asData?.value ?? const {},
        );
  final agents = <MentionCandidate>[
    for (final pubkey in source?.pubkeys ?? const <String>[])
      ?memberCandidates
          .where((c) => c.pubkey == pubkey && c.isAgent && c.isMember)
          .firstOrNull,
  ];
  final agentsKey = agents.map((a) => '${a.pubkey}:${a.label}').join(',');
  // The message whose prefill the viewer has settled, by editing or removing.
  final settledFor = useRef<String?>(null);

  // An edit away from the prefill makes the text the viewer's own draft.
  useEffect(() {
    void listener() {
      final prefill = prefillText.value;
      if (prefill == null || controller.text == prefill) return;
      prefillText.value = null;
      settledFor.value = source?.messageId;
    }

    controller.addListener(listener);
    return () => controller.removeListener(listener);
  }, [controller, source?.messageId]);

  useEffect(() {
    final prefill = prefillText.value;
    // The latest message has no agents to carry over (or they all left): take
    // back an untouched prefill, which describes an older message. While the
    // members are still loading, nothing is known yet, so leave it.
    final noAgents =
        source == null ||
        source.pubkeys.isEmpty ||
        (members != null && agents.isEmpty);
    if (noAgents) {
      if (prefill != null && controller.text == prefill) {
        prefillText.value = null;
        isModifyingText.value = true;
        try {
          controller.replaceDraft(TextEditingValue.empty, mentions: const []);
        } finally {
          isModifyingText.value = false;
        }
      }
      return null;
    }
    if (agents.isEmpty || settledFor.value == source.messageId) return null;
    // Fill only an empty composer, or replace this hook's own prefill.
    if (hasAttachments ||
        (controller.text.isNotEmpty && controller.text != prefill)) {
      return null;
    }
    // Each agent is one chip bound to its key; namesakes both read their
    // plain name and are told apart on the wire at send.
    final buffer = StringBuffer();
    final bindings = <_ComposerMention>[];
    for (final agent in agents) {
      final start = buffer.length;
      buffer.write('@${agent.label} ');
      bindings.add(
        _ComposerMention(
          start: start,
          end: start + 1 + agent.label.length,
          candidate: agent,
        ),
      );
    }
    final text = buffer.toString();
    // Namesake agents read the same text, so the same text is not the same
    // prefill: the keys and their places must match too.
    if (text == controller.text &&
        listEquals(
          [for (final m in controller.bindings) (m.start, m.end, m.pubkey)],
          [for (final m in bindings) (m.start, m.end, m.pubkey)],
        )) {
      return null;
    }
    prefillText.value = text;
    isModifyingText.value = true;
    try {
      controller.replaceDraft(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(offset: text.length),
        ),
        mentions: bindings,
      );
    } finally {
      isModifyingText.value = false;
    }
    return null;
  }, [source?.messageId, agentsKey, members == null, draftKey, hasAttachments]);
}
