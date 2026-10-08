import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'channel.dart';
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
    final error = useState<String?>(null);
    final actor = ref.watch(currentPubkeyProvider)?.toLowerCase();
    final roster = ref.watch(channelMembersProvider(channel.id));
    final channels = ref.watch(channelsProvider).asData?.value;
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
      if (viewer?.isElevated != true || member == null || member.isOwner) {
        return null;
      }
      return member;
    }

    final member = manageableMember(roster, actor, currentChannel);
    if (member == null) return const SizedBox.shrink();

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
      if (busy.value || !stillAllowed()) return;
      busy.value = true;
      error.value = null;
      try {
        if (role == null) {
          final label = ref
              .read(channelIdentityNamesProvider(channel.id))
              .labelFor(target);
          final confirmed = await showBuzzDialog<bool>(
            context: context,
            builder: (dialogContext) => AlertDialog(
              title: const Text('Remove from channel?'),
              content: Text('Remove $label from ${currentChannel.name}?'),
              actions: [
                TextButton(
                  onPressed: () => Navigator.of(dialogContext).pop(false),
                  child: const Text('Cancel'),
                ),
                TextButton(
                  onPressed: () => Navigator.of(dialogContext).pop(true),
                  child: const Text('Remove'),
                ),
              ],
            ),
          );
          if (!context.mounted || confirmed != true || !stillAllowed()) return;
          await ref
              .read(channelActionsProvider)
              .removeMember(channelId: channel.id, pubkey: target);
          if (context.mounted) Navigator.of(context).pop();
        } else {
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
            if (!member.isBot) ...[
              if (member.role != 'admin')
                AppListRow(
                  icon: BuzzIcons.shieldCheck,
                  title: 'Make channel admin',
                  onTap: busy.value ? null : () => perform(role: 'admin'),
                ),
              if (member.role == 'admin' || member.role == 'guest')
                AppListRow(
                  icon: BuzzIcons.user,
                  title: member.role == 'guest'
                      ? 'Make member'
                      : 'Change to member',
                  onTap: busy.value ? null : () => perform(role: 'member'),
                ),
            ],
            AppListRow(
              icon: BuzzIcons.userMinus,
              title: 'Remove from channel',
              titleColor: context.colors.error,
              onTap: busy.value ? null : () => perform(),
            ),
          ],
        ),
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
