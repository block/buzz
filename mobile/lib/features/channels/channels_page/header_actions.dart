part of '../channels_page.dart';

class _ChannelHeaderActions extends StatelessWidget {
  const _ChannelHeaderActions({
    required this.onSearch,
    required this.onInbox,
    required this.hasUnreadInbox,
    required this.onSettings,
  });

  final VoidCallback? onSearch;
  final VoidCallback? onInbox;
  final bool hasUnreadInbox;
  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      _surface(
        context,
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (onSearch != null)
              IconButton(
                tooltip: 'Search',
                icon: const Icon(BuzzIcons.search300),
                onPressed: onSearch,
              ),
            if (onInbox != null)
              Semantics(
                label: hasUnreadInbox ? 'Inbox, unread' : 'Inbox',
                button: true,
                onTap: onInbox,
                excludeSemantics: true,
                child: IconButton(
                  tooltip: 'Inbox',
                  onPressed: onInbox,
                  icon: Badge(
                    key: const ValueKey('inbox-header-badge'),
                    isLabelVisible: hasUnreadInbox,
                    child: const Icon(BuzzIcons.inbox300),
                  ),
                ),
              ),
          ],
        ),
      ),
      const SizedBox(width: Grid.xs),
      _surface(
        context,
        SizedBox(
          width: Grid.xl,
          height: Grid.xl,
          child: Center(
            child: ProfileAvatar(
              size: _kTopSectionProfileAvatarSize,
              showPresence: false,
              onTap: onSettings,
            ),
          ),
        ),
      ),
    ],
  );

  Widget _surface(BuildContext context, Widget child) => ClipRRect(
    borderRadius: BorderRadius.circular(Radii.full),
    child: ColoredBox(color: context.controlSurface, child: child),
  );
}
