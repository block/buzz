part of '../settings_page.dart';

class _StatusSection extends ConsumerWidget {
  const _StatusSection({this.onSetStatus});

  final void Function(BuildContext context)? onSetStatus;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final nsec = ref.watch(relayConfigProvider).nsec;
    return AppListCard(
      key: const ValueKey('status-identity-options'),
      dividerIndent: Grid.xs,
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const ValueKey('settings-set-status'),
          title: 'Set status',
          trailing: const _RowChevron(),
          onTap: () {
            unawaited(HapticFeedback.selectionClick());
            onSetStatus?.call(context);
          },
        ),
        if (nsec != null && nsec.isNotEmpty) _IdentityRow(nsec: nsec),
      ],
    );
  }
}
