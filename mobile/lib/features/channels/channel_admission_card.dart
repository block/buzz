part of 'channel_actions_sheet.dart';

/// Who joins an open channel and how: the role newcomers get, and whether the
/// relay joins every new community member to it. Every value comes from fresh
/// relay metadata, and a change shows only after the relay confirms it.
class _ChannelAdmissionCard extends HookConsumerWidget {
  const _ChannelAdmissionCard({
    required this.channel,
    required this.canEditJoinRole,
  });

  final Channel channel;

  /// Channel owners and admins choose the role newcomers get.
  final bool canEditJoinRole;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Only community owners and admins choose default channels, and only an
    // open, unarchived stream can be one.
    final managesDefault =
        channel.isStream &&
        !channel.isArchived &&
        managesCommunity(ref.watch(currentCommunityRoleProvider).value);
    final admissionAsync = ref.watch(channelAdmissionProvider(channel.id));
    final saving = useState(false);
    if (!canEditJoinRole && !managesDefault) return const SizedBox.shrink();

    final admission = admissionAsync.asData?.value;
    final ready = admission != null && !admissionAsync.isLoading;
    final canToggleDefault =
        managesDefault && ready && admission.canBeDefault && !saving.value;

    Future<void> save(Future<void> Function(ChannelActions) change) async {
      if (saving.value) return;
      saving.value = true;
      try {
        await change(ref.read(channelActionsProvider));
      } catch (error) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            SnackBar(
              content: Text(
                error is ChannelChangeUnconfirmed
                    ? 'Buzz couldn’t confirm the change yet. Check again '
                          'before trying again.'
                    : 'Couldn’t change channel access. Try again.',
              ),
            ),
            kind: ToastKind.error,
          );
        }
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    Future<void> chooseJoinRole() async {
      final current = admission?.joinRole;
      final role = await showBuzzModalBottomSheet<String>(
        context: context,
        title: 'New people join as',
        showDragHandle: true,
        builder: (sheetContext) => SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: SheetActionSection(
              children: [
                for (final (value, title, subtitle) in const [
                  ('member', 'Member', 'Can read and post'),
                  ('guest', 'Guest', 'Can read but not post'),
                ])
                  ListTile(
                    key: ValueKey('channel-join-role-$value'),
                    title: Text(title),
                    subtitle: Text(subtitle),
                    selected: current == value,
                    trailing: current == value
                        ? Icon(
                            BuzzIcons.check,
                            color: sheetContext.colors.primary,
                          )
                        : null,
                    onTap: () => Navigator.of(sheetContext).pop(value),
                  ),
              ],
            ),
          ),
        ),
      );
      if (role == null || role == current) return;
      await save((actions) => actions.setChannelJoinRole(channel.id, role));
    }

    void setDefault(bool enabled) => unawaited(
      save(
        (actions) => actions.setDefaultChannel(channel.id, enabled: enabled),
      ),
    );

    return AppListCard(
      key: const ValueKey('channel-details-access-card'),
      label: 'General access',
      verticalPadding: _channelDetailsSectionPadding,
      children: [
        if (admissionAsync.hasError && admission == null)
          AppListRow(
            key: const ValueKey('channel-access-retry'),
            icon: BuzzIcons.triangleAlert,
            title: 'Access settings unavailable',
            subtitle: 'Tap to try again.',
            onTap: () => ref.invalidate(channelAdmissionProvider(channel.id)),
          )
        else ...[
          if (canEditJoinRole)
            AppListRow(
              key: const ValueKey('channel-join-role-row'),
              icon: BuzzIcons.users,
              title: 'New people join as',
              value: switch (admission?.joinRole) {
                'guest' => 'Guest',
                'member' => 'Member',
                _ => null,
              },
              trailing: ready && !saving.value
                  ? const _ChannelDetailsChevron()
                  : const BuzzLoadingIndicator(
                      size: 20,
                      semanticLabel: 'Loading access settings',
                    ),
              onTap: ready && !saving.value ? chooseJoinRole : null,
            ),
          if (managesDefault)
            MergeSemantics(
              key: const ValueKey('channel-default-setting'),
              child: Stack(
                alignment: Alignment.centerRight,
                children: [
                  AppListRow(
                    key: const ValueKey('channel-default-row'),
                    icon: BuzzIcons.userPlus,
                    title: 'Default channel',
                    subtitle: 'New community members join automatically',
                    // Reserve room for the switch without growing the row.
                    trailing: const SizedBox(width: 60),
                    onTap: canToggleDefault
                        ? () => setDefault(!admission.autoJoin)
                        : null,
                  ),
                  Positioned.fill(
                    right: Grid.xs,
                    child: Align(
                      alignment: Alignment.centerRight,
                      child: Switch.adaptive(
                        value: admission?.autoJoin ?? false,
                        onChanged: canToggleDefault ? setDefault : null,
                      ),
                    ),
                  ),
                ],
              ),
            ),
        ],
      ],
    );
  }
}
