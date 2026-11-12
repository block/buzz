part of '../channel_detail_page.dart';

Future<void> _openMobileHuddle({
  required BuildContext context,
  required WidgetRef ref,
  required _HuddleInvite invite,
}) async {
  final config = ref.read(relayConfigProvider);
  final nsec = config.nsec;
  if (nsec == null || nsec.isEmpty) {
    ScaffoldMessenger.of(context).showFeedbackSnackBar(
      const SnackBar(content: Text('A paired identity is required.')),
      kind: ToastKind.generic,
    );
    return;
  }

  final active = ref.read(huddleSessionProvider);
  if (active.ephemeralChannelId == invite.ephemeralChannelId &&
      active.isInSession) {
    _showMobileHuddleCall(context: context, ref: ref, invite: invite);
    return;
  }

  try {
    final channel = ref.read(channelByIdProvider(invite.parentChannelId));
    if (ref.read(dmVoiceCandidateProvider(invite.parentChannelId)) != null) {
      ref.invalidate(ownedAgentDirectoryProvider);
      ref.invalidate(identityArchivesProvider);
      final agents = await ref.read(ownedCodexVoiceAgentsProvider.future);
      if (!context.mounted) return;
      final agent = dmVoiceAgent(
        channel!,
        ref.read(currentPubkeyProvider),
        agents,
      );
      if (agent == null) {
        throw StateError('This agent is no longer available for voice.');
      }
      {
        final members = await ref.refresh(
          channelMembersProvider(invite.ephemeralChannelId).future,
        );
        if (!context.mounted) return;
        if (!members.any((member) => member.pubkey == agent)) {
          final confirmed = await showConfirmation(
            context: context,
            title: 'Invite your agent?',
            message:
                'Your Codex agent is not invited to this Huddle. Invite it to hear this call? Keep Buzz open on the desktop running this agent.',
            confirmLabel: 'Invite and join',
          );
          if (!confirmed || !context.mounted) return;
          ref.invalidate(ownedAgentDirectoryProvider);
          ref.invalidate(identityArchivesProvider);
          final currentAgents = await ref.read(
            ownedCodexVoiceAgentsProvider.future,
          );
          if (!context.mounted) return;
          if (!currentAgents.contains(agent)) {
            throw StateError('This agent is no longer available for voice.');
          }
          await ref
              .read(channelActionsProvider)
              .addMembers(
                channelId: invite.ephemeralChannelId,
                pubkeys: [agent],
                role: 'bot',
              );
          if (!context.mounted) return;
        }
      }
    }
  } catch (error) {
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showFeedbackSnackBar(
      SnackBar(content: Text(_huddleActionError(error))),
      kind: ToastKind.error,
    );
    return;
  }

  final session = ref.read(huddleSessionProvider);
  _showMobileHuddleCall(context: context, ref: ref, invite: invite);
  if (session.ephemeralChannelId != invite.ephemeralChannelId ||
      session.phase == HuddleSessionPhase.idle ||
      session.phase == HuddleSessionPhase.failed) {
    unawaited(
      ref
          .read(mobileHuddleControllerProvider.notifier)
          .join(
            parentChannelId: invite.parentChannelId,
            ephemeralChannelId: invite.ephemeralChannelId,
            startedBy: invite.startedBy,
            startedEventId: invite.startedEventId,
          ),
    );
  }
}

final _agentConnectionTimeoutProvider = FutureProvider.autoDispose
    .family<bool, String>((ref, room) {
      final elapsed = Completer<bool>();
      final timer = Timer(
        const Duration(seconds: 15),
        () => elapsed.complete(true),
      );
      ref.onDispose(timer.cancel);
      return elapsed.future;
    });

class _HuddleAgentConnectionNotice extends ConsumerWidget {
  const _HuddleAgentConnectionNotice({required this.invite});
  final _HuddleInvite invite;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channel = ref.watch(channelByIdProvider(invite.parentChannelId));
    final session = ref.watch(huddleSessionProvider);
    if (channel?.isDm != true ||
        channel!.participantPubkeys.toSet().length != 2 ||
        !session.isConnected) {
      return const SizedBox.shrink();
    }
    final members =
        ref.watch(channelMembersProvider(invite.ephemeralChannelId)).value ??
        const <ChannelMember>[];
    final bots = members
        .where((member) => member.isBot)
        .map((member) => member.pubkey)
        .toSet();
    final eligible =
        ref.watch(ownedCodexVoiceAgentsProvider).value ?? const <String>{};
    bots.addAll(channel.participantPubkeys.where(eligible.contains));
    if (bots.isEmpty || bots.every(session.participantPubkeys.contains)) {
      return const SizedBox.shrink();
    }
    final expired =
        ref
            .watch(_agentConnectionTimeoutProvider(invite.ephemeralChannelId))
            .value ==
        true;
    return Semantics(
      liveRegion: true,
      child: Column(
        children: [
          Text(
            expired
                ? 'Your agent has not connected. Open Buzz on its desktop and check that the agent is running, then start a new Huddle.'
                : 'Connecting to your agent…',
            textAlign: TextAlign.center,
          ),
          if (expired)
            TextButton(
              onPressed: () async {
                await ref.read(mobileHuddleControllerProvider.notifier).leave();
                if (!context.mounted) return;
                await _startReplacementHuddle(
                  context: context,
                  ref: ref,
                  parentChannelId: invite.parentChannelId,
                );
              },
              child: const Text('Start a new Huddle'),
            ),
        ],
      ),
    );
  }
}
