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
    return PopupMenuButton<InboxFilter>(
      key: const ValueKey('activity-filter-menu'),
      onSelected: onChanged,
      itemBuilder: (context) => [
        for (final entry in _filterLabels.entries)
          PopupMenuItem(
            value: entry.key,
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
                Text(entry.value),
                const Spacer(),
                if (entry.key == InboxFilter.reminders && dueReminderCount > 0)
                  _CountBadge(count: dueReminderCount)
                else if (entry.key == InboxFilter.drafts && draftCount > 0)
                  _CountBadge(count: draftCount),
              ],
            ),
          ),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              _filterLabels[filter]!,
              style: context.textTheme.labelLarge?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(width: Grid.quarter),
            Icon(
              LucideIcons.chevronDown,
              size: 16,
              color: context.colors.onSurfaceVariant,
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
        color: context.colors.primary,
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
    return PopupMenuButton<String>(
      key: const ValueKey('activity-options-menu'),
      icon: const Icon(LucideIcons.ellipsis, size: 20),
      onSelected: (value) {
        if (value == 'unread-only') onUnreadOnlyChanged(!unreadOnly);
        if (value == 'mark-all-read') onMarkAllRead();
      },
      itemBuilder: (context) => [
        PopupMenuItem(
          value: 'unread-only',
          child: Row(
            children: [
              Expanded(child: Text(unreadOnly ? 'Show all' : 'Show unread')),
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
  }
}
