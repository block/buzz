part of '../channel_detail_page.dart';

Future<void> _showHuddleMembers(
  BuildContext context,
  WidgetRef ref,
  _HuddleInvite invite,
) async {
  final initial = ref.read(huddleSessionProvider);
  final config = ref.read(relayConfigProvider);
  final actions = ref.read(channelActionsProvider);
  bool current() {
    if (!context.mounted) return false;
    final session = ref.read(huddleSessionProvider);
    final activeConfig = ref.read(relayConfigProvider);
    return ref.read(relaySessionProvider).status == SessionStatus.connected &&
        activeConfig.wsUrl == config.wsUrl &&
        activeConfig.nsec == config.nsec &&
        session.ephemeralChannelId == invite.ephemeralChannelId &&
        session.parentChannelId == invite.parentChannelId &&
        session.currentPubkey == initial.currentPubkey &&
        session.wasAdmitted &&
        (session.phase == HuddleSessionPhase.connected ||
            session.phase == HuddleSessionPhase.interrupted);
  }

  if (!current()) return;
  final confirmed = await showConfirmation(
    context: context,
    title: 'Add members to this Huddle?',
    message:
        'People and agents you add can join this call and read its conversation. '
        'They will not be added to the original chat.',
    confirmLabel: 'Choose members',
  );
  if (!confirmed || !current()) return;
  try {
    final members =
        ref.read(channelMembersProvider(invite.ephemeralChannelId)).value ??
        const <ChannelMember>[];
    if (!context.mounted || !current()) return;
    await showAddChannelMembersSheet(
      context: context,
      channelId: invite.ephemeralChannelId,
      existingPubkeys: {
        for (final member in members) member.pubkey.toLowerCase(),
        if (initial.currentPubkey != null) initial.currentPubkey!.toLowerCase(),
      },
      onAddMembers: (selected) async {
        if (!current()) {
          throw AddMembersException({
            for (final person in selected)
              person.pubkey: 'This Huddle is no longer connected.',
          });
        }
        // A selected person may have joined while the picker was open. Do not
        // overwrite their current role with the role of a new invitation.
        final latestMembers = await ref.refresh(
          channelMembersProvider(invite.ephemeralChannelId).future,
        );
        final existing = {
          for (final member in latestMembers) member.pubkey.toLowerCase(),
        };
        final failures = <String, String>{};
        for (final person in selected) {
          if (existing.contains(person.pubkey.toLowerCase())) continue;
          if (!current()) {
            failures[person.pubkey] = 'This Huddle is no longer connected.';
            continue;
          }
          try {
            await actions.addMembers(
              channelId: invite.ephemeralChannelId,
              pubkeys: [person.pubkey],
              role: person.isAgent ? 'bot' : 'member',
            );
          } on AddMembersException catch (error) {
            failures.addAll(error.failures);
          } catch (error) {
            failures[person.pubkey] = _huddleActionError(error);
          }
        }
        if (failures.isNotEmpty) throw AddMembersException(failures);
      },
    );
  } catch (error) {
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showFeedbackSnackBar(
      SnackBar(content: Text(_huddleActionError(error))),
      kind: ToastKind.error,
    );
  }
}
