part of '../channels_page.dart';

class _CommunityMenuSheet extends StatelessWidget {
  const _CommunityMenuSheet({
    required this.canInvite,
    required this.onSwitchCommunity,
    this.invitePageBuilder,
    this.appearancePageBuilder,
  });

  final bool canInvite;
  final VoidCallback onSwitchCommunity;
  final WidgetBuilder? invitePageBuilder;
  final WidgetBuilder? appearancePageBuilder;

  @override
  Widget build(BuildContext context) {
    void openPage(WidgetBuilder builder) {
      final navigator = Navigator.of(context, rootNavigator: true);
      Navigator.of(context).pop();
      navigator.push(MaterialPageRoute<void>(builder: builder));
    }

    Future<void> openSwitcher() async {
      final route = ModalRoute.of(context);
      Navigator.of(context).pop();
      // Let the menu finish dismissing before the grid starts its entrance.
      if (route != null) await route.completed;
      onSwitchCommunity();
    }

    return SafeArea(
      child: SingleChildScrollView(
        key: const Key('community-switcher-sheet'),
        padding: const EdgeInsets.symmetric(vertical: Grid.xs),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (appearancePageBuilder != null ||
                (canInvite && invitePageBuilder != null))
              AppListCard(
                label: 'Community settings',
                children: [
                  if (canInvite && invitePageBuilder != null)
                    AppListRow(
                      icon: LucideIcons.userPlus,
                      title: 'Invite',
                      trailing: const Icon(LucideIcons.chevronRight, size: 18),
                      onTap: () => openPage(invitePageBuilder!),
                    ),
                  if (appearancePageBuilder != null)
                    AppListRow(
                      icon: LucideIcons.sunMoon,
                      title: 'Appearance',
                      trailing: const Icon(LucideIcons.chevronRight, size: 18),
                      onTap: () => openPage(appearancePageBuilder!),
                    ),
                ],
              ),
            AppListCard(
              children: [
                AppListRow(
                  key: const Key('community-menu-switch'),
                  icon: LucideIcons.arrowLeftRight,
                  title: 'Switch Community',
                  trailing: const Icon(LucideIcons.chevronRight, size: 18),
                  onTap: openSwitcher,
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

Future<void> _confirmRemoveCommunity(
  BuildContext context,
  WidgetRef ref,
  Community community, {
  required bool closeSheetAfterRemoval,
}) async {
  final messenger = ScaffoldMessenger.of(context);
  try {
    final confirmed = await showDestructiveConfirmation(
      context: context,
      title: 'Remove community?',
      message:
          'Are you sure you want to remove “${community.name}”? '
          'You can pair with it again later.',
      confirmLabel: 'Remove',
    );
    if (!confirmed || !context.mounted) return;
    await ref
        .read(communityListProvider.notifier)
        .removeCommunity(community.id);
    if (closeSheetAfterRemoval && context.mounted) {
      Navigator.of(context).pop();
    }
  } catch (e) {
    messenger.showSnackBar(
      SnackBar(content: Text('Failed to remove community: $e')),
    );
  }
}

class _CommunityIndicator extends ConsumerWidget {
  final VoidCallback onTap;

  const _CommunityIndicator({
    required this.onTap,
    required this.avatarKey,
    required this.hidden,
  });

  final GlobalKey avatarKey;
  final bool hidden;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activeAsync = ref.watch(activeCommunityProvider);

    final activeCommunity = activeAsync.value;

    return GestureDetector(
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Semantics(
        button: true,
        label: 'Community settings',
        child: Opacity(
          opacity: hidden ? 0 : 1,
          child: CommunityAvatar(
            key: avatarKey,
            name: activeCommunity?.name,
            relayUrl: activeCommunity?.relayUrl,
          ),
        ),
      ),
    );
  }
}

class _CommunityHeaderTitle extends ConsumerWidget {
  final TextStyle? style;
  final VoidCallback onTap;

  const _CommunityHeaderTitle({required this.onTap, this.style});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final name = ref.watch(activeCommunityProvider).value?.name;
    final title = name?.trim();
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: SizedBox.expand(
        child: Align(
          alignment: Alignment.centerLeft,
          child: Padding(
            padding: const EdgeInsets.only(left: Grid.xxs),
            child: Text(
              title == null || title.isEmpty ? 'Community' : title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: style,
            ),
          ),
        ),
      ),
    );
  }
}
