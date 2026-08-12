part of '../settings_page.dart';

class _CommunitySection extends ConsumerWidget {
  const _CommunitySection({required this.invitePageBuilder});

  final WidgetBuilder invitePageBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final role = ref.watch(currentCommunityRoleProvider).asData?.value;
    if (!canManageCommunityInvites(role)) return const SizedBox.shrink();

    return AppListCard(
      label: 'Community',
      children: [
        AppListRow(
          icon: LucideIcons.userPlus,
          title: 'Invite to community',
          trailing: const _RowChevron(),
          onTap: () => Navigator.of(
            context,
          ).push(MaterialPageRoute<void>(builder: invitePageBuilder)),
        ),
      ],
    );
  }
}
