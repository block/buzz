part of '../channels_page.dart';

class _CustomChannelSection extends StatelessWidget {
  final ChannelSection section;
  final List<Channel> channels;
  final Set<String> unreadChannelIds;
  final Map<String, int> unreadChannelCounts;
  final Set<String> mutedChannelIds;
  final String? currentPubkey;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final bool showTopDivider;
  final VoidCallback onToggle;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;
  final Future<void> Function(Channel channel) onSelectChannel;
  final void Function(Channel channel) onMarkChannelRead;

  const _CustomChannelSection({
    required this.section,
    required this.channels,
    required this.unreadChannelIds,
    required this.unreadChannelCounts,
    required this.mutedChannelIds,
    required this.currentPubkey,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.showTopDivider,
    required this.onToggle,
    required this.onRename,
    required this.onDelete,
    required this.onMoveUp,
    required this.onMoveDown,
    required this.onSelectChannel,
    required this.onMarkChannelRead,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (showTopDivider) const _SectionDivider(),
        _CustomSectionHeader(
          section: section,
          expanded: expanded,
          isFirst: isFirst,
          isLast: isLast,
          onToggle: onToggle,
          onRename: onRename,
          onDelete: onDelete,
          onMoveUp: onMoveUp,
          onMoveDown: onMoveDown,
        ),
        _AnimatedSectionBody(
          expanded: expanded,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final channel in channels)
                _ChannelTile(
                  channel: channel,
                  unreadCount: unreadChannelCounts[channel.id],
                  isUnread: unreadChannelIds.contains(channel.id),
                  isMuted: mutedChannelIds.contains(channel.id),
                  currentPubkey: currentPubkey,
                  onTap: () => onSelectChannel(channel),
                  onMarkRead: () => onMarkChannelRead(channel),
                  sectionId: section.id,
                ),
            ],
          ),
        ),
      ],
    );
  }
}

class _CustomSectionHeader extends ConsumerWidget {
  final ChannelSection section;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;

  const _CustomSectionHeader({
    required this.section,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.onRename,
    required this.onDelete,
    required this.onMoveUp,
    required this.onMoveDown,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sectionColor = context.colors.primary;
    final icon = section.icon;
    final customEmoji = icon == null
        ? null
        : _resolveCustomEmoji(icon, ref.watch(customEmojiListProvider));

    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.twelve,
          Grid.gutter,
          _kChannelRowVerticalPadding,
        ),
        child: Row(
          children: [
            SizedBox(
              width: _kChannelLeadingWidth,
              child: Align(
                alignment: Alignment.centerLeft,
                child: icon == null || icon.isEmpty
                    ? Icon(
                        LucideIcons.folder,
                        size: _kChannelIconSize,
                        color: sectionColor,
                      )
                    : customEmoji != null
                    ? CustomEmojiImage(
                        shortcode: customEmoji.shortcode,
                        url: customEmoji.url,
                        size: _kChannelIconSize,
                      )
                    : Text(
                        icon,
                        maxLines: 1,
                        overflow: TextOverflow.visible,
                        style: TextStyle(
                          fontSize: _kChannelIconSize,
                          height: 1,
                          color: sectionColor,
                        ),
                      ),
              ),
            ),
            const SizedBox(width: _kChannelLabelGap),
            Text(
              section.name,
              style: context.textTheme.bodyMedium?.copyWith(
                color: sectionColor,
                fontWeight: FontWeight.w600,
              ),
            ),
            const Spacer(),
            GestureDetector(
              onTapUp: (details) async {
                final overlay =
                    Overlay.of(context).context.findRenderObject()!
                        as RenderBox;
                final position = RelativeRect.fromRect(
                  details.globalPosition & Size.zero,
                  Offset.zero & overlay.size,
                );
                final value = await showMenu<String>(
                  context: context,
                  position: position,
                  items: [
                    const PopupMenuItem(value: 'rename', child: Text('Rename')),
                    PopupMenuItem(
                      value: 'move_up',
                      enabled: !isFirst,
                      child: const Text('Move Up'),
                    ),
                    PopupMenuItem(
                      value: 'move_down',
                      enabled: !isLast,
                      child: const Text('Move Down'),
                    ),
                    const PopupMenuItem(value: 'delete', child: Text('Delete')),
                  ],
                );
                switch (value) {
                  case 'rename':
                    onRename();
                  case 'move_up':
                    onMoveUp();
                  case 'move_down':
                    onMoveDown();
                  case 'delete':
                    onDelete();
                }
              },
              child: Icon(
                LucideIcons.ellipsisVertical,
                size: _kChannelIconSize,
                color: sectionColor,
              ),
            ),
            const SizedBox(width: Grid.quarter),
            _SectionChevron(expanded: expanded, color: sectionColor),
          ],
        ),
      ),
    );
  }
}

CustomEmoji? _resolveCustomEmoji(String icon, List<CustomEmoji> palette) {
  if (!icon.startsWith(':') || !icon.endsWith(':')) return null;
  final shortcode = normalizeShortcode(icon);
  if (shortcode == null) return null;
  for (final emoji in palette) {
    if (emoji.shortcode == shortcode) return emoji;
  }
  return null;
}

class _SectionNameDialog extends HookWidget {
  final String title;
  final String confirmLabel;
  final String initialValue;

  const _SectionNameDialog({
    required this.title,
    required this.confirmLabel,
    this.initialValue = '',
  });

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);

    void confirm() {
      final name = controller.text.trim();
      if (name.isNotEmpty) Navigator.of(context).pop(name);
    }

    return AlertDialog(
      title: Text(title),
      content: TextField(
        controller: controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (_) => confirm(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(onPressed: confirm, child: Text(confirmLabel)),
      ],
    );
  }
}

class _ChannelSection extends StatelessWidget {
  final String title;
  final IconData icon;
  final bool expanded;
  final VoidCallback onToggle;
  final List<Channel> channels;
  final bool showTopDivider;
  final Set<String> unreadChannelIds;
  final Map<String, int> unreadChannelCounts;
  final Set<String> mutedChannelIds;
  final String? currentPubkey;
  final String emptyLabel;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _ChannelSection({
    required this.title,
    required this.icon,
    required this.expanded,
    required this.onToggle,
    required this.channels,
    required this.showTopDivider,
    required this.unreadChannelIds,
    required this.unreadChannelCounts,
    required this.mutedChannelIds,
    required this.currentPubkey,
    required this.emptyLabel,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (showTopDivider) const _SectionDivider(),
        _SectionHeader(
          label: title,
          icon: icon,
          expanded: expanded,
          onToggle: onToggle,
        ),
        _AnimatedSectionBody(
          expanded: expanded,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (channels.isEmpty)
                Padding(
                  padding: const EdgeInsets.only(
                    left: _kChannelLabelInset,
                    right: _kChannelSectionInset,
                    top: Grid.half,
                    bottom: Grid.half,
                  ),
                  child: Text(
                    emptyLabel,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                )
              else
                for (final channel in channels)
                  _ChannelTile(
                    channel: channel,
                    unreadCount: unreadChannelCounts[channel.id],
                    isUnread: unreadChannelIds.contains(channel.id),
                    isMuted: mutedChannelIds.contains(channel.id),
                    currentPubkey: currentPubkey,
                    onTap: () => onSelectChannel(channel),
                    onMarkRead: null,
                    sectionId: null,
                  ),
            ],
          ),
        ),
      ],
    );
  }
}

class _EmptyState extends StatelessWidget {
  const _EmptyState();

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * 0.55,
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.messagesSquare,
              size: Grid.xl,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'No conversations yet',
              style: context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionDivider extends StatelessWidget {
  const _SectionDivider();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      child: Divider(
        height: 1,
        thickness: 1,
        indent: _kChannelSectionInset,
        endIndent: _kChannelSectionInset,
        color: context.colors.outlineVariant.withValues(alpha: 0.72),
      ),
    );
  }
}

class _SectionHeader extends StatelessWidget {
  final String label;
  final IconData icon;
  final bool expanded;
  final VoidCallback onToggle;

  const _SectionHeader({
    required this.label,
    required this.icon,
    required this.expanded,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    final sectionColor = context.colors.primary;

    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.twelve,
          Grid.gutter,
          _kChannelRowVerticalPadding,
        ),
        child: Row(
          children: [
            SizedBox(
              width: _kChannelLeadingWidth,
              child: Align(
                alignment: Alignment.centerLeft,
                child: Icon(icon, size: _kChannelIconSize, color: sectionColor),
              ),
            ),
            const SizedBox(width: _kChannelLabelGap),
            Text(
              label,
              style: context.textTheme.bodyMedium?.copyWith(
                color: sectionColor,
                fontWeight: FontWeight.w600,
              ),
            ),
            const Spacer(),
            _SectionChevron(expanded: expanded, color: sectionColor),
          ],
        ),
      ),
    );
  }
}

class _SectionChevron extends StatelessWidget {
  final bool expanded;
  final Color color;

  const _SectionChevron({required this.expanded, required this.color});

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.of(context).disableAnimations;

    return AnimatedRotation(
      turns: expanded ? 0 : -0.25,
      duration: reducedMotion ? Duration.zero : _kSectionExpandDuration,
      curve: _kSectionExpandCurve,
      child: Icon(
        LucideIcons.chevronDown,
        size: _kChannelIconSize,
        color: color,
      ),
    );
  }
}

class _AnimatedSectionBody extends HookWidget {
  final bool expanded;
  final Widget child;

  const _AnimatedSectionBody({required this.expanded, required this.child});

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final controller = useAnimationController(
      duration: reducedMotion ? Duration.zero : _kSectionExpandDuration,
      reverseDuration: reducedMotion
          ? Duration.zero
          : _kSectionCollapseDuration,
      initialValue: expanded ? 1 : 0,
    );
    final curvedAnimation = useMemoized(
      () => CurvedAnimation(
        parent: controller,
        curve: _kSectionExpandCurve,
        reverseCurve: _kSectionCollapseCurve,
      ),
      [controller],
    );
    final shouldRender = useState(expanded);

    useEffect(() => curvedAnimation.dispose, [curvedAnimation]);

    useEffect(() {
      void handleStatus(AnimationStatus status) {
        if (status == AnimationStatus.dismissed && !expanded) {
          shouldRender.value = false;
        }
      }

      controller.addStatusListener(handleStatus);
      return () => controller.removeStatusListener(handleStatus);
    }, [controller, expanded]);

    useEffect(() {
      controller.duration = reducedMotion
          ? Duration.zero
          : _kSectionExpandDuration;
      controller.reverseDuration = reducedMotion
          ? Duration.zero
          : _kSectionCollapseDuration;

      if (expanded) {
        shouldRender.value = true;
        if (reducedMotion) {
          controller.value = 1;
        } else {
          unawaited(controller.forward());
        }
      } else if (reducedMotion) {
        controller.value = 0;
        shouldRender.value = false;
      } else {
        unawaited(controller.reverse());
      }

      return null;
    }, [controller, expanded, reducedMotion]);

    return ClipRect(
      child: AnimatedBuilder(
        animation: curvedAnimation,
        child: shouldRender.value ? child : const SizedBox.shrink(),
        builder: (context, child) {
          final value = curvedAnimation.value.clamp(0.0, 1.0);
          final scaleY =
              _kSectionCollapsedScaleY +
              ((1 - _kSectionCollapsedScaleY) * value);

          return Align(
            alignment: Alignment.topCenter,
            heightFactor: value,
            child: Opacity(
              opacity: value,
              child: Transform.scale(
                alignment: Alignment.topCenter,
                scaleY: scaleY,
                child: ExcludeSemantics(
                  excluding: !expanded,
                  child: IgnorePointer(ignoring: !expanded, child: child),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}
