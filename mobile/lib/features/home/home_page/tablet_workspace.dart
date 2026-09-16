part of '../home_page.dart';

const double _tabletSidebarWidth = 300;
const double _tabletContentInset = Grid.half + Grid.quarter;

class _TabletWorkspace extends StatelessWidget {
  const _TabletWorkspace({
    required this.settingsPageBuilder,
    required this.hasUnreadInbox,
    required this.selectedDestination,
    required this.selectedChannel,
    required this.settingsTransitionProgress,
    required this.onDestinationSelected,
    required this.onChannelSelected,
    required this.onCommunityChanged,
    required this.onSelectedChannelUnavailable,
  });

  final WidgetBuilder settingsPageBuilder;
  final bool hasUnreadInbox;
  final int selectedDestination;
  final Channel? selectedChannel;
  final ValueNotifier<double> settingsTransitionProgress;
  final ValueChanged<int> onDestinationSelected;
  final ValueChanged<Channel> onChannelSelected;
  final ValueChanged<String?> onCommunityChanged;
  final VoidCallback onSelectedChannelUnavailable;

  @override
  Widget build(BuildContext context) {
    final gradient = context.appColors.topSectionGradient;
    final content = switch (selectedDestination) {
      0 when selectedChannel != null => ChannelDetailPage(
        key: ValueKey('tablet-channel-${selectedChannel!.id}'),
        channel: selectedChannel!,
      ),
      2 => const SearchPage(),
      _ => const ActivityPage(splitView: true),
    };

    return Scaffold(
      key: const ValueKey('tablet-workspace'),
      backgroundColor: Colors.transparent,
      resizeToAvoidBottomInset: false,
      body: DecoratedBox(
        decoration: BoxDecoration(
          color: gradient == null
              ? context.colors.surfaceContainerLowest
              : null,
          gradient: gradient,
        ),
        child: Row(
          children: [
            SizedBox(
              key: const ValueKey('tablet-workspace-sidebar'),
              width: _tabletSidebarWidth,
              child: ChannelsPage(
                settingsPageBuilder: settingsPageBuilder,
                onSettingsTransitionProgress: (progress) {
                  if (settingsTransitionProgress.value != progress) {
                    settingsTransitionProgress.value = progress;
                  }
                },
                selectedChannelId: selectedChannel?.id,
                onChannelSelected: onChannelSelected,
                onCommunityChanged: onCommunityChanged,
                onSelectedChannelUnavailable: onSelectedChannelUnavailable,
                workspaceHeader: _TabletWorkspaceDestinations(
                  selectedDestination: selectedChannel == null
                      ? selectedDestination
                      : 0,
                  hasUnreadInbox: hasUnreadInbox,
                  onDestinationSelected: onDestinationSelected,
                ),
              ),
            ),
            Expanded(
              child: Padding(
                key: const ValueKey('tablet-workspace-content-inset'),
                padding: const EdgeInsets.only(
                  top: _tabletContentInset,
                  right: _tabletContentInset,
                  bottom: _tabletContentInset,
                ),
                child: DecoratedBox(
                  key: const ValueKey('tablet-workspace-content-surface'),
                  decoration: BoxDecoration(
                    color: context.colors.surface,
                    borderRadius: BorderRadius.circular(Radii.dialog),
                    boxShadow: context.theme.brightness == Brightness.light
                        ? [
                            BoxShadow(
                              color: context.colors.outlineVariant.withValues(
                                alpha: 0.45,
                              ),
                              offset: const Offset(-1, -1),
                            ),
                            BoxShadow(
                              color: Colors.black.withValues(alpha: 0.07),
                              blurRadius: 4,
                            ),
                          ]
                        : null,
                  ),
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(Radii.dialog),
                    child: MediaQuery(
                      data: MediaQuery.of(context).copyWith(
                        padding: MediaQuery.paddingOf(context).copyWith(
                          top:
                              (MediaQuery.paddingOf(context).top -
                                      _tabletContentInset)
                                  .clamp(0, double.infinity),
                        ),
                      ),
                      child: content,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _TabletWorkspaceDestinations extends StatelessWidget {
  const _TabletWorkspaceDestinations({
    required this.selectedDestination,
    required this.hasUnreadInbox,
    required this.onDestinationSelected,
  });

  final int selectedDestination;
  final bool hasUnreadInbox;
  final ValueChanged<int> onDestinationSelected;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.xxs,
        Grid.xxs,
        Grid.xxs,
        Grid.half,
      ),
      child: Column(
        children: [
          _TabletWorkspaceDestination(
            icon: LucideIcons.inbox300,
            selectedIcon: LucideIcons.inbox500,
            label: 'Activity',
            selected: selectedDestination == 1,
            showBadge: hasUnreadInbox,
            onTap: () => onDestinationSelected(1),
          ),
          _TabletWorkspaceDestination(
            icon: LucideIcons.search300,
            selectedIcon: LucideIcons.search500,
            label: 'Search',
            selected: selectedDestination == 2,
            onTap: () => onDestinationSelected(2),
          ),
          Divider(
            height: Grid.xs,
            color: navigationSecondaryForeground(
              context,
            ).withValues(alpha: 0.16),
          ),
        ],
      ),
    );
  }
}

class _TabletWorkspaceDestination extends StatelessWidget {
  const _TabletWorkspaceDestination({
    required this.icon,
    required this.selectedIcon,
    required this.label,
    required this.selected,
    required this.onTap,
    this.showBadge = false,
  });

  final IconData icon;
  final IconData selectedIcon;
  final String label;
  final bool selected;
  final VoidCallback onTap;
  final bool showBadge;

  @override
  Widget build(BuildContext context) {
    final foreground = navigationPrimaryForeground(context);
    return Semantics(
      selected: selected,
      button: true,
      label: label,
      excludeSemantics: true,
      child: Material(
        color: selected
            ? context.colors.primaryContainer.withValues(alpha: 0.72)
            : Colors.transparent,
        borderRadius: BorderRadius.circular(Radii.md),
        child: InkWell(
          key: ValueKey('tablet-workspace-${label.toLowerCase()}'),
          onTap: onTap,
          borderRadius: BorderRadius.circular(Radii.md),
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 48),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Row(
                children: [
                  Stack(
                    clipBehavior: Clip.none,
                    children: [
                      Icon(
                        selected ? selectedIcon : icon,
                        size: 20,
                        color: foreground,
                      ),
                      if (showBadge)
                        Positioned(
                          top: -2,
                          right: -3,
                          child: Container(
                            width: 7,
                            height: 7,
                            decoration: BoxDecoration(
                              color: context.colors.error,
                              shape: BoxShape.circle,
                              border: Border.all(
                                color: context.colors.surface,
                                width: 1,
                              ),
                            ),
                          ),
                        ),
                    ],
                  ),
                  const SizedBox(width: Grid.twelve),
                  Text(
                    label,
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: foreground,
                      fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
