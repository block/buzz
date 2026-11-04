part of 'channel_actions_sheet.dart';

/// Who joins an open channel and how: the role newcomers get, and whether the
/// relay joins every new community member to it. Every value comes from fresh
/// relay metadata, and a change shows only after the relay confirms it. A
/// change the relay may have applied blocks further changes until a read-only
/// check confirms it, or until a read shows it unapplied and the same signed
/// change is sent again.
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
    final mayBeDefault = channel.isStream && !channel.isArchived;
    final communityRole = ref.watch(currentCommunityRoleProvider);
    final managesDefault =
        mayBeDefault && managesCommunity(communityRole.value);
    // Without the role the setting cannot be offered, so say why and retry.
    final roleUnknown =
        mayBeDefault && !managesDefault && communityRole.hasError;
    final admissionAsync = ref.watch(channelAdmissionProvider(channel.id));
    final uncertain = ref.watch(uncertainAdmissionChangeProvider(channel.id));
    final saving = useState(false);
    if (!canEditJoinRole && !managesDefault && !roleUnknown) {
      return const SizedBox.shrink();
    }

    final admission = admissionAsync.asData?.value;
    final ready =
        admission != null &&
        !admissionAsync.isLoading &&
        uncertain == null &&
        !saving.value;
    final canToggleDefault = managesDefault && ready && admission.canBeDefault;

    void showError(Object error) {
      if (!context.mounted) return;
      // Replace, not queue, so the message always matches the latest result.
      ScaffoldMessenger.of(context)
        ..hideCurrentSnackBar()
        ..showFeedbackSnackBar(
          SnackBar(
            content: Text(
              error is ChannelChangeUnconfirmed
                  ? 'Buzz couldn’t confirm the change yet. Check its status '
                        'before trying again.'
                  : 'Couldn’t change channel access. Try again.',
            ),
          ),
          kind: ToastKind.error,
        );
    }

    Future<void> save(Future<void> Function(ChannelActions) change) async {
      if (saving.value) return;
      saving.value = true;
      try {
        await change(ref.read(channelActionsProvider));
      } catch (error) {
        showError(error);
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
        if (roleUnknown)
          AppListRow(
            key: const ValueKey('channel-community-role-retry'),
            icon: BuzzIcons.triangleAlert,
            title: 'Default channel unavailable',
            subtitle: 'Couldn’t check your community role. Tap to try again.',
            onTap: () => ref.invalidate(communityMembershipProvider),
          ),
        if (uncertain != null)
          AppListRow(
            key: const ValueKey('channel-access-check'),
            icon: BuzzIcons.triangleAlert,
            title: 'Change not confirmed',
            subtitle: 'It may have saved. Tap to check its status.',
            trailing: saving.value
                ? const BuzzLoadingIndicator(
                    size: 20,
                    semanticLabel: 'Checking save status',
                  )
                : null,
            onTap: saving.value
                ? null
                : () => save(
                    (actions) => actions.checkAdmissionChange(channel.id),
                  ),
          ),
        if (uncertain != null && uncertain.unseen)
          AppListRow(
            key: const ValueKey('channel-access-resend'),
            icon: BuzzIcons.refreshCw,
            title: 'Send again',
            subtitle: 'The relay doesn’t show the change yet.',
            onTap: saving.value
                ? null
                : () => save(
                    (actions) => actions.resendAdmissionChange(channel.id),
                  ),
          ),
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
              trailing: ready
                  ? const _ChannelDetailsChevron()
                  : uncertain != null
                  ? null
                  : const BuzzLoadingIndicator(
                      size: 20,
                      semanticLabel: 'Loading access settings',
                    ),
              onTap: ready ? chooseJoinRole : null,
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
