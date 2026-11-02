import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/confirmation_dialog.dart';
import 'channel.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import 'channel_access_provider.dart';
import 'member_access_controls.dart';
import 'channel_identity_names_provider.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';

/// Channel-scoped management rows embedded below a member's profile actions.
class ChannelMemberProfileActions extends HookConsumerWidget {
  const ChannelMemberProfileActions({
    super.key,
    required this.channel,
    required this.pubkey,
  });

  final Channel channel;
  final String pubkey;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final busy = useState(false);
    final accessController = ref.watch(channelAccessProvider.notifier);
    final pendingAccess = ref.watch(channelAccessProvider)[channel.id];
    final error = useState<String?>(null);
    final actor = ref.watch(currentPubkeyProvider)?.toLowerCase();
    final roster = ref.watch(channelMembersProvider(channel.id));
    final channels = ref.watch(channelsProvider).asData?.value;
    final viewer = roster.asData?.value
        .where((member) => member.pubkey.toLowerCase() == actor)
        .firstOrNull;
    final owners =
        viewer != null &&
            !viewer.isElevated &&
            !channel.isSolo &&
            !channel.isDm &&
            !channel.isArchived
        ? ref.watch(agentOwnersProvider).asData?.value ??
              const <String, String>{}
        : const <String, String>{};
    final currentChannel =
        channels
            ?.where((candidate) => candidate.id == channel.id)
            .firstOrNull ??
        channel;
    final target = pubkey.toLowerCase();

    ChannelMember? manageableMember(
      AsyncValue<List<ChannelMember>> snapshot,
      String? currentActor,
      Channel current,
    ) {
      if (current.isDm ||
          current.isArchived ||
          snapshot.isLoading ||
          snapshot.hasError ||
          currentActor == null ||
          target == currentActor) {
        return null;
      }
      final members = snapshot.asData?.value ?? const <ChannelMember>[];
      final viewer = members
          .where((m) => m.pubkey.toLowerCase() == currentActor)
          .firstOrNull;
      final member = members
          .where((m) => m.pubkey.toLowerCase() == target)
          .firstOrNull;
      final ownsAgent =
          !current.isSolo && viewer != null && owners[target] == currentActor;
      if ((viewer?.isElevated != true && !ownsAgent) ||
          member == null ||
          !const {'admin', 'member', 'guest', 'bot'}.contains(member.role)) {
        return null;
      }
      return member;
    }

    final member = manageableMember(roster, actor, currentChannel);
    if (member == null) return const SizedBox.shrink();
    final canChangeRoles =
        roster.asData?.value
            .where((m) => m.pubkey.toLowerCase() == actor)
            .firstOrNull
            ?.isElevated ==
        true;

    bool stillAllowed() {
      final latest =
          ref
              .read(channelsProvider)
              .asData
              ?.value
              .where((candidate) => candidate.id == channel.id)
              .firstOrNull ??
          currentChannel;
      return ref.read(currentPubkeyProvider)?.toLowerCase() == actor &&
          manageableMember(
                ref.read(channelMembersProvider(channel.id)),
                actor,
                latest,
              )?.role ==
              member.role;
    }

    Future<void> perform({String? role}) async {
      if (busy.value || pendingAccess != null || !stillAllowed()) return;
      busy.value = true;
      error.value = null;
      try {
        if (role == null) {
          final label = ref
              .read(channelIdentityNamesProvider(channel.id))
              .labelFor(target);
          final confirmed = await showDestructiveConfirmation(
            context: context,
            title: 'Remove from channel?',
            message: 'Remove $label from ${currentChannel.name}?',
            confirmLabel: 'Remove',
          );
          if (!context.mounted || confirmed != true || !stillAllowed()) return;
          await ref
              .read(channelActionsProvider)
              .removeMember(channelId: channel.id, pubkey: target);
          if (context.mounted) Navigator.of(context).pop();
        } else {
          final confirmed = await showDialog<bool>(
            context: context,
            builder: (dialogContext) => AlertDialog(
              title: const Text('Change access?'),
              content: Text(
                'Set this person to ${memberAccessLabel(role).toLowerCase()}?',
              ),
              actions: [
                TextButton(
                  autofocus: true,
                  onPressed: () => Navigator.pop(dialogContext, false),
                  child: const Text('Cancel'),
                ),
                FilledButton(
                  onPressed: () => Navigator.pop(dialogContext, true),
                  child: const Text('Change access'),
                ),
              ],
            ),
          );
          if (!context.mounted || confirmed != true || !stillAllowed()) return;
          await ref
              .read(channelActionsProvider)
              .changeMemberRole(
                channelId: channel.id,
                pubkey: target,
                role: role,
              );
        }
      } catch (_) {
        if (context.mounted) {
          error.value = role == null
              ? 'Could not remove this person. Please try again.'
              : 'Could not change this role. Please try again.';
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        AppListCard(
          horizontalPadding: 0,
          children: [
            if (canChangeRoles && !currentChannel.isSolo) ...[
              if (member.role != 'admin')
                AppListRow(
                  icon: BuzzIcons.shieldCheck,
                  title: 'Make channel admin',
                  onTap: busy.value || pendingAccess != null
                      ? null
                      : () => perform(role: 'admin'),
                ),
              if (member.role == 'admin' || member.role == 'guest')
                AppListRow(
                  icon: BuzzIcons.user,
                  title: 'Participating',
                  onTap: busy.value || pendingAccess != null
                      ? null
                      : () => perform(role: 'member'),
                ),
            ],
            if (canChangeRoles &&
                !currentChannel.isSolo &&
                member.role != 'guest')
              AppListRow(
                icon: BuzzIcons.shield,
                title: 'Viewing only',
                onTap: busy.value || pendingAccess != null
                    ? null
                    : () => perform(role: 'guest'),
              ),
            AppListRow(
              icon: BuzzIcons.userMinus,
              title: 'Remove from channel',
              titleColor: context.colors.error,
              onTap: busy.value || pendingAccess != null
                  ? null
                  : () => perform(),
            ),
          ],
        ),
        if (pendingAccess != null) ...[
          TextButton(
            onPressed: busy.value
                ? null
                : () async {
                    busy.value = true;
                    try {
                      await accessController.checkSave(channel.id);
                    } catch (reason) {
                      if (context.mounted) error.value = reason.toString();
                    } finally {
                      if (context.mounted) busy.value = false;
                    }
                  },
            child: const Text('Check save status'),
          ),
          TextButton(
            onPressed: busy.value
                ? null
                : () async {
                    if (await confirmDiscardAccessSave(context) != true ||
                        !context.mounted) {
                      return;
                    }
                    busy.value = true;
                    try {
                      await accessController.discardSave(
                        channel.id,
                        pendingAccess,
                      );
                      if (context.mounted) error.value = null;
                    } catch (reason) {
                      if (context.mounted) error.value = reason.toString();
                    } finally {
                      if (context.mounted) busy.value = false;
                    }
                  },
            child: const Text('Discard and reload'),
          ),
        ],
        if (busy.value)
          Semantics(
            liveRegion: true,
            label: 'Updating channel member',
            child: const ExcludeSemantics(
              child: Center(child: Text('Updating…')),
            ),
          ),
        if (error.value != null)
          Semantics(
            liveRegion: true,
            label: error.value!,
            child: ExcludeSemantics(
              child: Text(
                error.value!,
                style: context.textTheme.bodyMedium?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ),
          ),
      ],
    );
  }
}
