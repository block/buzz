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
                      icon: BuzzIcons.userPlus,
                      title: 'Invite',
                      trailing: const Icon(BuzzIcons.chevronRight, size: 18),
                      onTap: () => openPage(invitePageBuilder!),
                    ),
                  if (appearancePageBuilder != null)
                    AppListRow(
                      icon: BuzzIcons.sunMoon,
                      title: 'Appearance',
                      trailing: const Icon(BuzzIcons.chevronRight, size: 18),
                      onTap: () => openPage(appearancePageBuilder!),
                    ),
                ],
              ),
            // An internal build reaches only its organization's community.
            if (!OrganizationEnvironment.enabled)
              AppListCard(
                children: [
                  AppListRow(
                    key: const Key('community-menu-switch'),
                    icon: BuzzIcons.arrowLeftRight,
                    title: 'Switch Community',
                    trailing: const Icon(BuzzIcons.chevronRight, size: 18),
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
  final account = ref.read(builderlabAccountProvider);
  final sync = ref.read(knownCommunitySyncProvider);
  try {
    final subject = sync == null ? null : await account.retainedSubject();
    if (!context.mounted) return;
    final retainsDestination = (ref.read(communityListProvider).value ?? [])
        .any(
          (item) =>
              item.id != community.id &&
              knownCommunityAddress(item.relayUrl) ==
                  knownCommunityAddress(community.relayUrl),
        );
    final confirmed = await showDestructiveConfirmation(
      context: context,
      title: 'Remove community?',
      message: subject == null
          ? 'Are you sure you want to remove “${community.pickerName}”? You can pair with it again later.'
          : retainsDestination
          ? 'Remove this identity for “${community.pickerName}” from this device? Another saved identity still uses this community, so it stays in your account list.'
          : 'Remove “${community.pickerName}” from this device and request removal from your Builder Lab community list? Account removal syncs when connected.',
      confirmLabel: 'Remove',
    );
    if (!confirmed ||
        !context.mounted ||
        (sync != null && await account.retainedSubject() != subject)) {
      return;
    }
    await ref
        .read(communityListProvider.notifier)
        .removeCommunity(
          community.id,
          confirmedSubject: subject,
          accountConfirmed: true,
        );
    if (closeSheetAfterRemoval && context.mounted) {
      Navigator.of(context).pop();
    }
  } catch (e) {
    messenger.showFeedbackSnackBar(
      SnackBar(content: Text('Failed to remove community: $e')),
      kind: ToastKind.error,
    );
  }
}

Future<void> _confirmRemoveAccountCommunity(
  BuildContext context,
  WidgetRef ref,
  AddCommunityDeepLink community,
) async {
  final account = ref.read(builderlabAccountProvider);
  final sync = ref.read(knownCommunitySyncProvider);
  if (sync == null) return;
  try {
    final subject = await account.retainedSubject();
    if (!context.mounted || subject == null) return;
    final confirmed = await showDestructiveConfirmation(
      context: context,
      title: 'Remove community?',
      message:
          'Remove “${community.name}” from your Builder Lab community list? Removal syncs when connected.',
      confirmLabel: 'Remove',
    );
    if (!confirmed ||
        !context.mounted ||
        await account.retainedSubject() != subject) {
      return;
    }
    await sync.remove(community.relayUrl, expectedSubject: subject);
    sync.trigger();
  } catch (_) {
    if (context.mounted) {
      ScaffoldMessenger.of(context).showFeedbackSnackBar(
        const SnackBar(
          content: Text('Could not save this removal. Please try again.'),
        ),
        kind: ToastKind.error,
      );
    }
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
          child: ClipRRect(
            borderRadius: BorderRadius.circular(Radii.full),
            child: ColoredBox(
              color: context.controlSurface,
              child: SizedBox.square(
                dimension: Grid.xl,
                child: Center(
                  child: CommunityAvatar(
                    key: avatarKey,
                    size: _kTopSectionProfileAvatarSize,
                    name: activeCommunity?.name,
                    relayUrl: activeCommunity?.relayUrl,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
