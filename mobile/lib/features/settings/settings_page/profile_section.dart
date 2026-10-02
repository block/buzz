part of '../settings_page.dart';

class _ProfileSection extends StatelessWidget {
  const _ProfileSection({
    required this.profileEditPageBuilder,
    this.onSetStatus,
    this.onEditDisplayName,
    this.onEditProfileDescription,
  });

  final WidgetBuilder profileEditPageBuilder;
  final void Function(BuildContext context)? onSetStatus;
  final Future<void> Function(BuildContext context)? onEditDisplayName;
  final Future<void> Function(BuildContext context)? onEditProfileDescription;

  @override
  Widget build(BuildContext context) => AppListCard(
    key: const ValueKey('edit-profile-options'),
    label: 'Edit profile',
    dividerIndent: Grid.xs,
    verticalPadding: Grid.twelve,
    children: [
      AppListRow(
        key: const ValueKey('edit-profile-status'),
        title: 'Set status',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          onSetStatus?.call(context);
        },
      ),
      AppListRow(
        key: const ValueKey('edit-profile-display-name'),
        title: 'Display name',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          unawaited(onEditDisplayName?.call(context));
        },
      ),
      AppListRow(
        key: const ValueKey('edit-profile-description'),
        title: 'Profile description',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          unawaited(onEditProfileDescription?.call(context));
        },
      ),
      AppListRow(
        key: const ValueKey('edit-profile-photo'),
        title: 'Photo',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          Navigator.of(
            context,
          ).push(immediatePageRoute<void>(builder: profileEditPageBuilder));
        },
      ),
    ],
  );
}
