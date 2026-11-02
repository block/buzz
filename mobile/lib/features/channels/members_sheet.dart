import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/sheet_action_section.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../profile/user_status.dart';
import '../profile/user_status_cache_provider.dart';
import 'agent_activity/agent_activity_sheet.dart';
import 'agent_activity/working_bots_provider.dart';
import 'channel.dart';
import '../../shared/mentions/agent_identity_provider.dart'
    show agentOwnersProvider;
import 'member_access_controls.dart';
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
    final userCache = ref.watch(userCacheProvider);
    bool isAgent(ChannelMember member) =>
        member.isBot || userCache[member.pubkey.toLowerCase()]?.isAgent == true;
    final people = allMembers.where((member) => !isAgent(member)).toList();
    final bots = allMembers.where(isAgent).toList();
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

    final owners = !canManage && currentMember != null && !channel.isSolo
        ? ref.watch(agentOwnersProvider).asData?.value ??
              const <String, String>{}
        : const <String, String>{};

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
              if (people.isNotEmpty)
                _MemberSection(
                  label: 'People · ${people.length}',
                  itemCount: people.length,
                  itemBuilder: (context, index) {
                    final member = people[index];
                    return _MemberTile(
                      member: member,
                      currentPubkey: currentPubkey,
                      profile: userCache[member.pubkey.toLowerCase()],
                      canManage: canManage,
                      canRemoveOwn:
                          !channel.isSolo &&
                          currentMember != null &&
                          owners[member.pubkey.toLowerCase()] ==
                              currentPubkey?.toLowerCase(),
                      isSelf:
                          member.pubkey.toLowerCase() ==
                          currentPubkey?.toLowerCase(),
                      channelId: channel.id,
                      userStatus: statusCache[member.pubkey.toLowerCase()],
                      onActivityTap: onMemberTap == null
                          ? null
                          : () => onMemberTap!(context, member.pubkey),
                    );
                  },
                ),
              if (bots.isNotEmpty)
                _MemberSection(
                  label: 'Agents · ${bots.length}',
                  semanticIndexOffset: people.length,
                  itemCount: bots.length,
                  itemBuilder: (context, index) {
                    final bot = bots[index];
                    return _MemberTile(
                      member: bot,
                      currentPubkey: currentPubkey,
                      profile: userCache[bot.pubkey.toLowerCase()],
                      canManage: canManage,
                      canRemoveOwn:
                          !channel.isSolo &&
                          currentMember != null &&
                          owners[bot.pubkey.toLowerCase()] ==
                              currentPubkey?.toLowerCase(),
                      isSelf: false,
                      channelId: channel.id,
                      isWorking: typingBotPubkeys.contains(
                        bot.pubkey.toLowerCase(),
                      ),
                      onViewActivity: () => openActivity(bot),
                      onActivityTap:
                          typingBotPubkeys.contains(bot.pubkey.toLowerCase())
                          ? () => openActivity(bot)
                          : null,
                    );
                  },
                ),
              if (people.isEmpty && bots.isEmpty)
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

String _roleLabel(String role) {
  return memberAccessLabel(role);
}

class _MemberTile extends ConsumerWidget {
  final ChannelMember member;
  final String? currentPubkey;
  final UserProfile? profile;
  final bool canManage;
  final bool canRemoveOwn;
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
    this.canRemoveOwn = false,
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
    final showManagementActions =
        (canManage || canRemoveOwn) && !isSelf && !member.isOwner;
    final showMenu = showManagementActions || onViewActivity != null;

    return ListTile(
      leading: _MemberAvatar(
        avatarUrl: profile?.avatarUrl,
        initial: initial,
        isAgent: member.isBot || profile?.isAgent == true,
      ),
      title: Text(label),
      subtitle: isWorking
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
          : userStatus != null && !userStatus!.isEmpty
          ? Text(
              '${userStatus!.emoji.isNotEmpty ? '${userStatus!.emoji} ' : ''}${userStatus!.text}',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            )
          : !const {'owner', 'admin', 'guest'}.contains(member.role)
          ? null
          : Text(
              _roleLabel(member.role),
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
      isScrollControlled: true,
      builder: (sheetContext) => SafeArea(
        child: SingleChildScrollView(
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
              if (showManagementActions)
                MemberAccessControls(
                  channelId: channelId,
                  target: member.pubkey.toLowerCase(),
                  label: label,
                ),
              const SizedBox(height: Grid.xxs),
            ],
          ),
        ),
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
