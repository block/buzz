part of 'home_page.dart';

class _TabletWorkspaceSidebar extends ConsumerWidget {
  const _TabletWorkspaceSidebar({
    required this.selectedPageIndex,
    required this.selectedChannelId,
    required this.onDestinationSelected,
    required this.onChannelSelected,
    required this.settingsPageBuilder,
    required this.destinations,
  });

  final int selectedPageIndex;
  final String? selectedChannelId;
  final ValueChanged<int> onDestinationSelected;
  final Future<void> Function(Channel channel) onChannelSelected;
  final WidgetBuilder settingsPageBuilder;
  final List<_HomeDestination> destinations;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channelsAsync = ref.watch(channelsProvider);
    final profile = ref.watch(profileProvider).asData?.value;
    final currentPubkey = profile?.pubkey;
    final colorScheme = context.colors;

    return SizedBox(
      key: const Key('tablet-workspace-sidebar'),
      width: HomePage._workspaceSidebarWidth,
      child: ColoredBox(
        color: colorScheme.surfaceContainerLow,
        child: SafeArea(
          right: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Padding(
                padding: EdgeInsets.fromLTRB(
                  Grid.twelve,
                  Grid.twelve,
                  Grid.xs,
                  Grid.xxs,
                ),
                child: CommunitySwitcherButton(),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
                child: Column(
                  children: [
                    for (final destination in destinations)
                      _TabletSidebarDestinationTile(
                        destination: destination,
                        selected:
                            selectedChannelId == null &&
                            destination.pageIndex == selectedPageIndex,
                        onTap: () =>
                            onDestinationSelected(destination.pageIndex),
                      ),
                  ],
                ),
              ),
              Divider(
                height: Grid.xs,
                indent: Grid.xs,
                endIndent: Grid.xs,
                color: colorScheme.outlineVariant.withValues(alpha: 0.72),
              ),
              Expanded(
                child: channelsAsync.when(
                  loading: () => const _TabletDirectoryStatus(loading: true),
                  error: (_, _) => _TabletDirectoryStatus(
                    onRetry: () =>
                        ref.read(channelsProvider.notifier).refresh(),
                  ),
                  data: (channels) => channels.isEmpty
                      ? const _TabletDirectoryStatus()
                      : CustomScrollView(
                          slivers: [
                            ChannelDirectorySliver(
                              channels: channels,
                              currentPubkey: currentPubkey,
                              directMessagesLabel: 'Direct Messages',
                              selectedChannelId: selectedChannelId,
                              onSelectChannel: onChannelSelected,
                            ),
                          ],
                        ),
                ),
              ),
              Material(
                color: Colors.transparent,
                child: InkWell(
                  key: const Key('tablet-profile-footer'),
                  onTap: () => Navigator.of(
                    context,
                  ).push(MaterialPageRoute<void>(builder: settingsPageBuilder)),
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(
                      Grid.twelve,
                      Grid.twelve,
                      Grid.xs,
                      Grid.twelve,
                    ),
                    child: Row(
                      children: [
                        const ProfileAvatar(showPresence: true),
                        const SizedBox(width: Grid.half + Grid.quarter),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Text(
                                profile?.label ?? 'Your profile',
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: context.textTheme.labelLarge?.copyWith(
                                  fontWeight: FontWeight.w600,
                                ),
                              ),
                              if (profile?.nip05Handle case final handle?)
                                Text(
                                  handle,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: context.textTheme.bodySmall?.copyWith(
                                    color: colorScheme.onSurfaceVariant,
                                  ),
                                ),
                            ],
                          ),
                        ),
                        Icon(
                          LucideIcons.settings,
                          size: 18,
                          color: colorScheme.onSurfaceVariant,
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _TabletSidebarDestinationTile extends StatelessWidget {
  const _TabletSidebarDestinationTile({
    required this.destination,
    required this.selected,
    required this.onTap,
  });

  final _HomeDestination destination;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colorScheme = context.colors;
    final foreground = selected
        ? colorScheme.onPrimaryContainer
        : colorScheme.onSurfaceVariant;

    return Material(
      color: selected ? colorScheme.primaryContainer : Colors.transparent,
      borderRadius: BorderRadius.circular(Radii.md),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        key: ValueKey('tablet-destination-${destination.label}'),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.twelve,
            vertical: Grid.xxs,
          ),
          child: Row(
            children: [
              Icon(
                selected ? destination.selectedIcon : destination.icon,
                size: HomePage._tabIconSize,
                color: foreground,
              ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: Text(
                  destination.label,
                  style: context.textTheme.labelLarge?.copyWith(
                    color: foreground,
                    fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _TabletDirectoryStatus extends StatelessWidget {
  const _TabletDirectoryStatus({this.loading = false, this.onRetry});

  final bool loading;
  final VoidCallback? onRetry;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.only(bottom: Grid.xs),
      children: [
        const _TabletDirectorySectionLabel(
          icon: LucideIcons.hash,
          label: 'Channels',
        ),
        if (!loading)
          Padding(
            padding: const EdgeInsets.only(
              left: 50,
              right: Grid.xs,
              bottom: Grid.xxs,
            ),
            child: Text(
              onRetry == null ? 'No channels yet' : 'Could not load channels',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        const _TabletDirectorySectionLabel(
          icon: LucideIcons.messagesSquare,
          label: 'Direct Messages',
        ),
        if (loading)
          Padding(
            padding: const EdgeInsets.only(left: 50, top: Grid.xxs),
            child: Text(
              'Loading conversations…',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          )
        else if (onRetry != null)
          Align(
            alignment: Alignment.centerLeft,
            child: Padding(
              padding: const EdgeInsets.only(left: 50),
              child: TextButton(onPressed: onRetry, child: const Text('Retry')),
            ),
          )
        else
          Padding(
            padding: const EdgeInsets.only(left: 50, right: Grid.xs),
            child: Text(
              'No direct messages yet',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
      ],
    );
  }
}

class _TabletDirectorySectionLabel extends StatelessWidget {
  const _TabletDirectorySectionLabel({required this.icon, required this.label});

  final IconData icon;
  final String label;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.twelve,
        Grid.gutter,
        Grid.xxs,
      ),
      child: Row(
        children: [
          SizedBox(
            width: 22,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Icon(icon, size: 18, color: context.colors.primary),
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: contentListTitleTextStyle.copyWith(
                color: context.colors.primary,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
