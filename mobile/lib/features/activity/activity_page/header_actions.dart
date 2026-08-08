part of '../activity_page.dart';

const _filterLabels = <InboxFilter, String>{
  InboxFilter.all: 'All',
  InboxFilter.mention: 'Mentions',
  InboxFilter.thread: 'Threads',
  InboxFilter.needsAction: 'Needs Action',
  InboxFilter.activity: 'Activity',
  InboxFilter.agentActivity: 'Agents',
  InboxFilter.reminders: 'Reminders',
  InboxFilter.drafts: 'Drafts',
};

class _ActivityActionsPill extends StatelessWidget {
  final InboxFilter filter;
  final int dueReminderCount;
  final int draftCount;
  final bool unreadOnly;
  final int unreadCount;
  final ValueChanged<InboxFilter> onFilterChanged;
  final ValueChanged<bool> onUnreadOnlyChanged;
  final VoidCallback onMarkAllRead;

  const _ActivityActionsPill({
    required this.filter,
    required this.dueReminderCount,
    required this.draftCount,
    required this.unreadOnly,
    required this.unreadCount,
    required this.onFilterChanged,
    required this.onUnreadOnlyChanged,
    required this.onMarkAllRead,
  });

  @override
  Widget build(BuildContext context) => ClipRRect(
    borderRadius: BorderRadius.circular(Radii.full),
    child: DecoratedBox(
      decoration: BoxDecoration(
        color: context.colors.primaryContainer,
        borderRadius: BorderRadius.circular(Radii.full),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.quarter),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _FilterMenuButton(
              filter: filter,
              dueReminderCount: dueReminderCount,
              draftCount: draftCount,
              onChanged: onFilterChanged,
            ),
            _InboxOptionsButton(
              unreadOnly: unreadOnly,
              unreadCount: unreadCount,
              onUnreadOnlyChanged: onUnreadOnlyChanged,
              onMarkAllRead: onMarkAllRead,
            ),
          ],
        ),
      ),
    ),
  );
}

/// Compact filter dropdown replacing the old chip rail — mirrors desktop's
/// inbox filter menu (`FILTER_OPTIONS`).
class _FilterMenuButton extends StatelessWidget {
  final InboxFilter filter;
  final int dueReminderCount;
  final int draftCount;
  final ValueChanged<InboxFilter> onChanged;

  const _FilterMenuButton({
    required this.filter,
    required this.dueReminderCount,
    required this.draftCount,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return Builder(
      builder: (buttonContext) => InkWell(
        key: const ValueKey('activity-filter-menu'),
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: () async {
          final selected = await showAnchoredPopover<InboxFilter>(
            context: buttonContext,
            width: 240,
            alignment: AnchoredPopoverAlignment.start,
            offset: const Offset(0, Grid.half),
            menuPadding: const EdgeInsets.symmetric(vertical: Grid.half),
            surfaceKey: const ValueKey('activity-filter-popover'),
            items: [
              for (final entry in _filterLabels.entries)
                PopupMenuItem(
                  value: entry.key,
                  height: Grid.xl,
                  padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
                  child: Row(
                    children: [
                      SizedBox(
                        width: Grid.sm,
                        child: entry.key == filter
                            ? Icon(
                                LucideIcons.check,
                                size: 16,
                                color: context.colors.primary,
                              )
                            : null,
                      ),
                      Expanded(
                        child: Text(
                          entry.value,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.labelLarge?.copyWith(
                            color: context.colors.onSurface,
                          ),
                        ),
                      ),
                      if (entry.key == InboxFilter.reminders &&
                          dueReminderCount > 0)
                        _CountBadge(count: dueReminderCount)
                      else if (entry.key == InboxFilter.drafts &&
                          draftCount > 0)
                        _CountBadge(count: draftCount),
                    ],
                  ),
                ),
            ],
          );
          if (buttonContext.mounted && selected != null) onChanged(selected);
        },
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: Grid.xl),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  _filterLabels[filter]!,
                  style: context.textTheme.labelLarge?.copyWith(
                    color: navigationPrimaryForeground(context),
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const SizedBox(width: Grid.quarter),
                Icon(
                  LucideIcons.chevronDown,
                  size: 16,
                  color: navigationPrimaryForeground(context),
                ),
                if (dueReminderCount > 0 || draftCount > 0) ...[
                  const SizedBox(width: Grid.quarter),
                  Container(
                    width: 6,
                    height: 6,
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      color: context.colors.primary,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _CountBadge extends StatelessWidget {
  final int count;

  const _CountBadge({required this.count});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.half + Grid.quarter,
        vertical: Grid.quarter,
      ),
      decoration: BoxDecoration(
        color: navigationPrimaryForeground(context),
        borderRadius: BorderRadius.circular(Grid.xxs),
      ),
      child: Text(
        '$count',
        style: context.textTheme.labelSmall?.copyWith(
          color: context.colors.onPrimary,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}

/// Overflow menu with the unread-only toggle and mark-all-read, mirroring
/// desktop's inbox options popover.
class _InboxOptionsButton extends StatelessWidget {
  final bool unreadOnly;
  final int unreadCount;
  final ValueChanged<bool> onUnreadOnlyChanged;
  final VoidCallback onMarkAllRead;

  const _InboxOptionsButton({
    required this.unreadOnly,
    required this.unreadCount,
    required this.onUnreadOnlyChanged,
    required this.onMarkAllRead,
  });

  @override
  Widget build(BuildContext context) {
    return Builder(
      builder: (buttonContext) => IconButton(
        key: const ValueKey('activity-options-menu'),
        tooltip: 'Activity options',
        color: navigationPrimaryForeground(context),
        padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
        constraints: const BoxConstraints.tightFor(
          width: Grid.xl,
          height: Grid.xl,
        ),
        icon: const Icon(LucideIcons.ellipsis, size: 20),
        onPressed: () async {
          final selected = await showAnchoredPopover<String>(
            context: buttonContext,
            width: 216,
            alignment: AnchoredPopoverAlignment.end,
            surfaceKey: const ValueKey('activity-options-popover'),
            items: [
              PopupMenuItem(
                value: 'unread-only',
                child: Row(
                  children: [
                    Expanded(
                      child: Text(unreadOnly ? 'Show all' : 'Show unread'),
                    ),
                    if (unreadOnly)
                      Icon(
                        LucideIcons.check,
                        size: 16,
                        color: context.colors.primary,
                      ),
                  ],
                ),
              ),
              PopupMenuItem(
                value: 'mark-all-read',
                enabled: unreadCount > 0,
                child: Row(
                  children: [
                    const Expanded(child: Text('Mark all as read')),
                    if (unreadCount > 0)
                      Text(
                        '$unreadCount',
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                  ],
                ),
              ),
            ],
          );
          if (!buttonContext.mounted || selected == null) return;
          if (selected == 'unread-only') onUnreadOnlyChanged(!unreadOnly);
          if (selected == 'mark-all-read') onMarkAllRead();
        },
      ),
    );
  }
}
