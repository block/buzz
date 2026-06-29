import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../profile/user_status.dart';
import '../profile/user_status_cache_provider.dart';
import 'agent_activity/agent_activity_sheet.dart';
import 'agent_activity/working_bots_provider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';

class MembersSheet extends HookConsumerWidget {
  final Channel channel;
  final String? currentPubkey;

  const MembersSheet({
    super.key,
    required this.channel,
    required this.currentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final membersAsync = ref.watch(channelMembersProvider(channel.id));
    final allMembers = membersAsync.asData?.value ?? const <ChannelMember>[];
    final people = allMembers.where((member) => !member.isBot).toList();
    final bots = allMembers.where((member) => member.isBot).toList();
    final userCache = ref.watch(userCacheProvider);
    final typingBotPubkeys = ref.watch(workingBotPubkeysProvider(channel.id));
    final statusCache = ref.watch(userStatusCacheProvider);

    // Determine if the current user can manage members.
    final currentMember = allMembers.cast<ChannelMember?>().firstWhere(
      (m) => m!.pubkey.toLowerCase() == currentPubkey?.toLowerCase(),
      orElse: () => null,
    );
    final canManage =
        currentMember != null &&
        currentMember.isElevated &&
        !channel.isArchived;

    void openActivity(ChannelMember bot) {
      final navigator = Navigator.of(context);
      navigator.pop();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!navigator.mounted) return;
        showModalBottomSheet<void>(
          context: navigator.context,
          isScrollControlled: true,
          showDragHandle: true,
          builder: (_) => AgentActivitySheet(
            channelId: channel.id,
            agentPubkey: bot.pubkey,
          ),
        );
      });
    }

    // Preload profiles for all members so avatars appear.
    useEffect(() {
      if (allMembers.isNotEmpty) {
        ref
            .read(userCacheProvider.notifier)
            .preload(allMembers.map((m) => m.pubkey).toList());
        // Track user statuses for people (not bots).
        final peoplePubkeys = allMembers
            .where((m) => !m.isBot)
            .map((m) => m.pubkey)
            .toList();
        if (peoplePubkeys.isNotEmpty) {
          ref.read(userStatusCacheProvider.notifier).track(peoplePubkeys);
        }
      }
      return null;
    }, [allMembers.length]);

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom,
      ),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Members', style: context.textTheme.titleMedium),
            const SizedBox(height: Grid.xxs),
            if (!channel.isDm) ...[const Divider(height: 1)],
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 400),
              child: membersAsync.when(
                data: (_) => ListView(
                  shrinkWrap: true,
                  padding: const EdgeInsets.only(top: Grid.xxs),
                  children: [
                    if (people.isNotEmpty) ...[
                      _SectionLabel(label: 'People — ${people.length}'),
                      for (final member in people)
                        _MemberTile(
                          member: member,
                          currentPubkey: currentPubkey,
                          profile: userCache[member.pubkey.toLowerCase()],
                          canManage: canManage,
                          isSelf:
                              member.pubkey.toLowerCase() ==
                              currentPubkey?.toLowerCase(),
                          channelId: channel.id,
                          userStatus: statusCache[member.pubkey.toLowerCase()],
                        ),
                    ],
                    if (bots.isNotEmpty) ...[
                      const SizedBox(height: Grid.xxs),
                      _SectionLabel(label: 'Bots — ${bots.length}'),
                      for (final bot in bots)
                        _MemberTile(
                          member: bot,
                          currentPubkey: currentPubkey,
                          profile: userCache[bot.pubkey.toLowerCase()],
                          canManage: canManage,
                          isSelf: false,
                          channelId: channel.id,
                          isWorking: typingBotPubkeys.contains(
                            bot.pubkey.toLowerCase(),
                          ),
                          onViewActivity: () => openActivity(bot),
                          onActivityTap:
                              typingBotPubkeys.contains(
                                bot.pubkey.toLowerCase(),
                              )
                              ? () => openActivity(bot)
                              : null,
                        ),
                    ],
                    if (people.isEmpty && bots.isEmpty)
                      Center(
                        child: Text(
                          'No members found.',
                          style: context.textTheme.bodySmall?.copyWith(
                            color: context.colors.onSurfaceVariant,
                          ),
                        ),
                      ),
                  ],
                ),
                loading: () => const Center(child: CircularProgressIndicator()),
                error: (error, _) => Center(
                  child: Text(
                    error.toString(),
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.error,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionLabel extends StatelessWidget {
  final String label;

  const _SectionLabel({required this.label});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: Grid.half, bottom: Grid.half),
      child: Text(
        label.toUpperCase(),
        style: context.textTheme.labelSmall?.copyWith(
          color: context.colors.onSurfaceVariant,
          fontWeight: FontWeight.w600,
          letterSpacing: 0.8,
        ),
      ),
    );
  }
}

const _changeableRoles = ['admin', 'member', 'guest'];

String _roleLabel(String role) {
  if (role.isEmpty) return 'Member';
  return '${role[0].toUpperCase()}${role.substring(1)}';
}

class _MemberTile extends ConsumerWidget {
  final ChannelMember member;
  final String? currentPubkey;
  final UserProfile? profile;
  final bool canManage;
  final bool isSelf;
  final String channelId;
  final bool isWorking;
  final VoidCallback? onActivityTap;
  final VoidCallback? onViewActivity;
  final UserStatus? userStatus;

  const _MemberTile({
    required this.member,
    required this.currentPubkey,
    required this.profile,
    required this.canManage,
    required this.isSelf,
    required this.channelId,
    this.isWorking = false,
    this.onActivityTap,
    this.onViewActivity,
    this.userStatus,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final label = isSelf
        ? 'You'
        : (profile?.displayName?.trim().isNotEmpty == true
              ? profile!.displayName!.trim()
              : member.labelFor(currentPubkey));
    final initial = label.substring(0, 1).toUpperCase();
    final showManagementActions = canManage && !isSelf && !member.isOwner;
    final showMenu = showManagementActions || onViewActivity != null;

    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: _MemberAvatar(avatarUrl: profile?.avatarUrl, initial: initial),
      title: Text(label),
      subtitle: isWorking
          ? Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  width: 10,
                  height: 10,
                  child: CircularProgressIndicator(
                    strokeWidth: 1.5,
                    color: context.appColors.success,
                  ),
                ),
                const SizedBox(width: Grid.half),
                Text(
                  'Working\u2026',
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.appColors.success,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ],
            )
          : userStatus != null && !userStatus!.isEmpty
          ? Text(
              '${userStatus!.emoji.isNotEmpty ? '${userStatus!.emoji} ' : ''}${userStatus!.text}',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            )
          : Text(
              _roleLabel(member.role),
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
      trailing: showMenu
          ? IconButton(
              icon: const Icon(LucideIcons.ellipsis, size: 18),
              onPressed: () => _showMemberActions(
                context,
                ref,
                showManagementActions: showManagementActions,
              ),
              visualDensity: VisualDensity.compact,
            )
          : null,
      onTap: onActivityTap,
    );
  }

  void _showMemberActions(
    BuildContext context,
    WidgetRef ref, {
    required bool showManagementActions,
  }) {
    final label = isSelf
        ? 'You'
        : (profile?.displayName?.trim().isNotEmpty == true
              ? profile!.displayName!.trim()
              : member.labelFor(currentPubkey));
    final canChangeRole = showManagementActions && !member.isBot;
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Text(label, style: context.textTheme.titleSmall),
            ),
            const SizedBox(height: Grid.xxs),
            if (onViewActivity != null)
              ListTile(
                leading: Icon(
                  LucideIcons.activity,
                  size: 18,
                  color: context.colors.primary,
                ),
                title: const Text('View activity'),
                onTap: () {
                  Navigator.of(sheetContext).pop();
                  WidgetsBinding.instance.addPostFrameCallback((_) {
                    onViewActivity?.call();
                  });
                },
              ),
            if (showManagementActions) ...[
              if (canChangeRole) ...[
                const SizedBox(height: Grid.xxs),
                _RoleSelector(
                  selectedRole: member.role,
                  onChanged: (role) async {
                    Navigator.of(sheetContext).pop();
                    await ref
                        .read(channelActionsProvider)
                        .changeMemberRole(
                          channelId: channelId,
                          pubkey: member.pubkey,
                          role: role,
                        );
                  },
                ),
                const SizedBox(height: Grid.xs),
              ],
              ListTile(
                leading: Icon(
                  LucideIcons.userMinus,
                  size: 18,
                  color: context.colors.error,
                ),
                title: Text(
                  'Remove from channel',
                  style: TextStyle(color: context.colors.error),
                ),
                onTap: () async {
                  Navigator.of(context).pop();
                  final confirmed = await showDialog<bool>(
                    context: context,
                    builder: (context) => AlertDialog(
                      title: const Text('Remove member'),
                      content: Text('Remove $label from this channel?'),
                      actions: [
                        TextButton(
                          onPressed: () => Navigator.of(context).pop(false),
                          child: const Text('Cancel'),
                        ),
                        TextButton(
                          onPressed: () => Navigator.of(context).pop(true),
                          child: Text(
                            'Remove',
                            style: TextStyle(color: context.colors.error),
                          ),
                        ),
                      ],
                    ),
                  );
                  if (confirmed == true) {
                    await ref
                        .read(channelActionsProvider)
                        .removeMember(
                          channelId: channelId,
                          pubkey: member.pubkey,
                        );
                  }
                },
              ),
            ],
            const SizedBox(height: Grid.xxs),
          ],
        ),
      ),
    );
  }
}

class _RoleSelector extends StatelessWidget {
  final String selectedRole;
  final ValueChanged<String> onChanged;

  const _RoleSelector({required this.selectedRole, required this.onChanged});

  @override
  Widget build(BuildContext context) {
    final hasKnownRole = _changeableRoles.contains(selectedRole);

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'Role',
            style: context.textTheme.labelMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.xxs),
          SizedBox(
            width: double.infinity,
            child: SegmentedButton<String>(
              segments: [
                for (final role in _changeableRoles)
                  ButtonSegment<String>(
                    value: role,
                    label: Text(_roleLabel(role)),
                  ),
              ],
              selected: hasKnownRole ? {selectedRole} : const <String>{},
              emptySelectionAllowed: !hasKnownRole,
              showSelectedIcon: false,
              style: ButtonStyle(
                visualDensity: VisualDensity.compact,
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                textStyle: WidgetStatePropertyAll(context.textTheme.labelSmall),
              ),
              onSelectionChanged: (roles) {
                if (roles.isEmpty) return;
                final role = roles.single;
                if (role == selectedRole) return;
                onChanged(role);
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _MemberAvatar extends HookWidget {
  final String? avatarUrl;
  final String initial;

  const _MemberAvatar({required this.avatarUrl, required this.initial});

  @override
  Widget build(BuildContext context) {
    final failed = useState(false);

    useEffect(() {
      failed.value = false;
      return null;
    }, [avatarUrl]);

    final url = avatarUrl;
    if (url == null || failed.value) {
      return CircleAvatar(child: Text(initial));
    }
    return CircleAvatar(
      backgroundImage: NetworkImage(url),
      onBackgroundImageError: (_, _) => failed.value = true,
      child: null,
    );
  }
}
