import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/identity_archives/identity_archives.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/sheet_action_section.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/confirmation_dialog.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../profile/user_status.dart';
import '../profile/user_status_cache_provider.dart';
import 'agent_activity/agent_activity_sheet.dart';
import 'agent_activity/working_bots_provider.dart';
import 'channel.dart';
import 'channel_identity_names_provider.dart';
import 'channel_management_provider.dart';

part 'members_sheet/member_section.dart';

class MembersSheet extends HookConsumerWidget {
  final Channel channel;
  final String? currentPubkey;
  final void Function(BuildContext context, String pubkey)? onMemberTap;

  const MembersSheet({
    super.key,
    required this.channel,
    required this.currentPubkey,
    this.onMemberTap,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final membersAsync = ref.watch(channelMembersProvider(channel.id));
    final allMembers = membersAsync.asData?.value ?? const <ChannelMember>[];
    // Known-archived members leave People and Agents for their own section
    // after both: archived people, then archived agents, in the relay's order.
    final archived = ref.watch(hiddenArchivedIdentitiesProvider);
    bool isArchived(ChannelMember member) =>
        archived.contains(member.pubkey.toLowerCase());
    final people = [
      for (final member in allMembers)
        if (!member.isBot && !isArchived(member)) member,
    ];
    final bots = [
      for (final member in allMembers)
        if (member.isBot && !isArchived(member)) member,
    ];
    final archivedMembers = [
      for (final bot in [false, true])
        for (final member in allMembers)
          if (member.isBot == bot && isArchived(member)) member,
    ];
    final userCache = ref.watch(userCacheProvider);
    final typingBotPubkeys = ref.watch(workingBotPubkeysProvider(channel.id));
    final statusCache = ref.watch(userStatusCacheProvider);
    final bottomClearance = Grid.md + MediaQuery.viewPaddingOf(context).bottom;

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
        showBuzzModalBottomSheet<void>(
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

    Widget tile(ChannelMember member) {
      final key = member.pubkey.toLowerCase();
      final working = member.isBot && typingBotPubkeys.contains(key);
      return _MemberTile(
        member: member,
        currentPubkey: currentPubkey,
        profile: userCache[key],
        canManage: canManage,
        isArchived: isArchived(member),
        isSelf: !member.isBot && key == currentPubkey?.toLowerCase(),
        channelId: channel.id,
        userStatus: member.isBot ? null : statusCache[key],
        isWorking: working,
        onViewActivity: member.isBot ? () => openActivity(member) : null,
        onActivityTap: member.isBot
            ? (working ? () => openActivity(member) : null)
            : onMemberTap == null
            ? null
            : () => onMemberTap!(context, member.pubkey),
      );
    }

    return Padding(
      key: const ValueKey('members-sheet-content-padding'),
      padding: EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, 0),
      child: ConstrainedBox(
        key: const ValueKey('members-sheet-viewport'),
        constraints: BoxConstraints(maxHeight: 400 + bottomClearance),
        child: membersAsync.when(
          data: (_) => CustomScrollView(
            key: const ValueKey('members-sheet-list'),
            semanticChildCount: allMembers.length,
            shrinkWrap: true,
            slivers: [
              const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
              for (final (label, members, offset) in [
                ('People', people, 0),
                ('Agents', bots, people.length),
                ('Archived', archivedMembers, people.length + bots.length),
              ])
                if (members.isNotEmpty)
                  _MemberSection(
                    label: '$label · ${members.length}',
                    semanticIndexOffset: offset,
                    itemCount: members.length,
                    itemBuilder: (context, index) => tile(members[index]),
                  ),
              if (allMembers.isEmpty)
                SliverToBoxAdapter(
                  child: Center(
                    child: Text(
                      'No members found.',
                      style: context.textTheme.bodySmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ),
                ),
              SliverToBoxAdapter(child: SizedBox(height: bottomClearance)),
            ],
          ),
          loading: () => const Center(
            child: BuzzLoadingIndicator(
              size: 44,
              semanticLabel: 'Loading members',
            ),
          ),
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
    );
  }
}

const _changeableRoles = ['admin', 'member', 'guest'];

String _roleLabel(String role) {
  if (role == 'bot') return 'Agent';
  if (role.isEmpty) return 'Member';
  return '${role[0].toUpperCase()}${role.substring(1)}';
}

class _MemberTile extends ConsumerWidget {
  final ChannelMember member;
  final String? currentPubkey;
  final UserProfile? profile;
  final bool canManage;

  /// Known-archived: the row says so, in text a screen reader reads too.
  final bool isArchived;
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
    this.isArchived = false,
    required this.isSelf,
    required this.channelId,
    this.isWorking = false,
    this.onActivityTap,
    this.onViewActivity,
    this.userStatus,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final name = isSelf
        ? 'You'
        : (profile?.displayName?.trim().isNotEmpty == true
              ? profile!.displayName!.trim()
              : member.labelFor(currentPubkey));
    final label = _watchContextualLabel(ref);
    // Named members initial from their name; unnamed ones stay keyed to the
    // hex public key so the compact-npub label doesn't render `N` for all.
    final hasName = profile?.displayName?.trim().isNotEmpty == true;
    final initial = isSelf || hasName
        ? name[0].toUpperCase()
        : (member.pubkey.isNotEmpty ? member.pubkey[0].toUpperCase() : '?');
    final showManagementActions = canManage && !isSelf && !member.isOwner;
    final showMenu = showManagementActions || onViewActivity != null;

    return ListTile(
      leading: _MemberAvatar(
        avatarUrl: profile?.avatarUrl,
        initial: initial,
        isAgent: member.isBot || profile?.isAgent == true,
      ),
      title: Text(label),
      subtitle: isWorking && !isArchived
          ? Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                BuzzLoadingIndicator(
                  size: 14,
                  color: context.appColors.success,
                  semanticLabel: 'Agent working',
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
          : !isArchived && userStatus != null && !userStatus!.isEmpty
          ? Text(
              '${userStatus!.emoji.isNotEmpty ? '${userStatus!.emoji} ' : ''}${userStatus!.text}',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            )
          : Text(
              isArchived
                  ? 'Archived \u00b7 ${_roleLabel(member.role)}'
                  : _roleLabel(member.role),
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
      trailing: showMenu
          ? IconButton(
              icon: const Icon(BuzzIcons.ellipsis, size: 18),
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

  /// 'You' for the viewer; otherwise the channel's contextual label.
  String _watchContextualLabel(WidgetRef ref) =>
      isSelf ? 'You' : watchChannelIdentityLabel(ref, channelId, member.pubkey);

  void _showMemberActions(
    BuildContext context,
    WidgetRef ref, {
    required bool showManagementActions,
  }) {
    final label = isSelf
        ? 'You'
        : ref
              .read(channelIdentityNamesProvider(channelId))
              .labelFor(member.pubkey);
    showBuzzModalBottomSheet<void>(
      context: context,
      title: label,
      showDragHandle: true,
      builder: (sheetContext) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (onViewActivity != null)
              SheetActionSection(
                horizontalPadding: Grid.gutter,
                children: [
                  ListTile(
                    leading: Icon(
                      BuzzIcons.activity,
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
                ],
              ),
            if (showManagementActions) ...[
              const SizedBox(height: Grid.xxs),
              _RoleSelector(
                selectedRole: member.isBot ? 'member' : member.role,
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
              SheetActionSection(
                horizontalPadding: Grid.gutter,
                children: [
                  ListTile(
                    leading: Icon(
                      BuzzIcons.userMinus,
                      size: 18,
                      color: context.colors.error,
                    ),
                    title: Text(
                      'Remove from channel',
                      style: TextStyle(color: context.colors.error),
                    ),
                    onTap: () async {
                      Navigator.of(context).pop();
                      final confirmed = await showDestructiveConfirmation(
                        context: context,
                        title: 'Remove member',
                        message: 'Remove $label from this channel?',
                        confirmLabel: 'Remove',
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

class _MemberAvatar extends StatelessWidget {
  final String? avatarUrl;
  final String initial;
  final bool isAgent;

  const _MemberAvatar({
    required this.avatarUrl,
    required this.initial,
    required this.isAgent,
  });

  @override
  Widget build(BuildContext context) {
    return AvatarImage(
      imageUrl: avatarUrl,
      radius: 20,
      fallback: Text(initial),
      isAgent: isAgent,
    );
  }
}
