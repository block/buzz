part of '../channels_page.dart';

const _kQuickActionsTabMotionDuration = Duration(milliseconds: 220);
const _kQuickActionsTabMotionCurve = Cubic(0.77, 0, 0.175, 1);
const _kQuickActionsHiddenOverlap = Grid.half;
const _kQuickActionsHiddenScale = 0.8;

/// Explicit placement geometry for [ChannelQuickActionsLauncher].
class ChannelQuickActionsPlacement {
  final double rightInset;
  final double closedBottomInset;
  final double openBottomInset;
  final double hiddenHorizontalOffset;

  const ChannelQuickActionsPlacement._({
    required this.rightInset,
    required this.closedBottomInset,
    required this.openBottomInset,
    required this.hiddenHorizontalOffset,
  });

  /// Aligns the launcher beside a centered floating bottom navigation bar.
  factory ChannelQuickActionsPlacement.besideBottomNavigation({
    required double screenWidth,
    required double navigationBarHeight,
    required double navigationBarBottomGap,
    required double navigationBarWidth,
    required double systemBottomInset,
    required double rightInset,
  }) {
    final navigationBottomInset = max(
      systemBottomInset,
      navigationBarBottomGap,
    );
    final verticalCentering = (navigationBarHeight - _kMorphClosedSize) / 2;
    final closedBottomInset = navigationBottomInset + verticalCentering;
    final navigationBarRight = (screenWidth + navigationBarWidth) / 2;
    final launcherRight = screenWidth - rightInset;

    return ChannelQuickActionsPlacement._(
      rightInset: rightInset,
      closedBottomInset: closedBottomInset,
      openBottomInset:
          closedBottomInset +
          navigationBarHeight +
          Grid.xxs -
          verticalCentering,
      hiddenHorizontalOffset:
          navigationBarRight - launcherRight - _kQuickActionsHiddenOverlap,
    );
  }

  /// Anchors the launcher to the bottom-trailing corner of its content pane.
  factory ChannelQuickActionsPlacement.bottomTrailing({
    required double systemBottomInset,
    double bottomGap = Grid.gutter,
    double rightInset = Grid.gutter,
  }) {
    final closedBottomInset = max(systemBottomInset, bottomGap);
    return ChannelQuickActionsPlacement._(
      rightInset: rightInset,
      closedBottomInset: closedBottomInset,
      openBottomInset: closedBottomInset + _kMorphClosedSize + Grid.xxs,
      hiddenHorizontalOffset: 0,
    );
  }
}

/// Places and transitions the channel quick-actions button within its pane.
class ChannelQuickActionsLauncher extends HookConsumerWidget {
  /// Whether the launcher should be visible on the current destination.
  final bool visible;

  /// Placement supplied by the navigation shell that owns the surrounding UI.
  final ChannelQuickActionsPlacement placement;

  const ChannelQuickActionsLauncher({
    super.key,
    required this.visible,
    required this.placement,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final quickActionsOpen = useState(false);
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final effectiveOpen = visible && quickActionsOpen.value;

    useEffect(() {
      if (!visible && quickActionsOpen.value) {
        quickActionsOpen.value = false;
      }
      return null;
    }, [visible]);

    Future<void> openChannel(Channel channel) async {
      if (!context.mounted) return;
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(channel: channel),
        ),
      );
    }

    Future<void> selectQuickAction(_QuickAction action) async {
      quickActionsOpen.value = false;
      if (!reducedMotion) {
        await Future<void>.delayed(_kMorphCloseDuration);
      }
      if (!context.mounted) return;

      switch (action) {
        case _QuickAction.createChannel:
          final created = await showModalBottomSheet<Channel>(
            context: context,
            constraints: _quickActionSheetConstraints(context),
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) => const _CreateChannelSheet(channelType: 'stream'),
          );
          if (created != null && context.mounted) {
            await openChannel(created);
          }
        case _QuickAction.newDm:
          final opened = await showModalBottomSheet<Channel>(
            context: context,
            constraints: _quickActionSheetConstraints(context),
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) =>
                _NewDirectMessageSheet(currentPubkey: currentPubkey),
          );
          if (opened != null && context.mounted) {
            await openChannel(opened);
          }
      }
    }

    return Stack(
      fit: StackFit.expand,
      clipBehavior: Clip.none,
      children: [
        if (quickActionsOpen.value)
          Positioned.fill(
            child: Semantics(
              button: true,
              label: 'Close quick actions',
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: () => quickActionsOpen.value = false,
              ),
            ),
          ),
        AnimatedPositioned(
          duration: reducedMotion
              ? Duration.zero
              : _kQuickActionsTabMotionDuration,
          curve: _kQuickActionsTabMotionCurve,
          right: placement.rightInset,
          bottom: effectiveOpen
              ? placement.openBottomInset
              : placement.closedBottomInset,
          child: IgnorePointer(
            ignoring: !visible,
            child: ExcludeSemantics(
              excluding: !visible,
              child: TweenAnimationBuilder<double>(
                key: const Key('channel-quick-actions-motion'),
                tween: Tween(end: visible ? 0 : 1),
                duration: reducedMotion
                    ? Duration.zero
                    : _kQuickActionsTabMotionDuration,
                curve: _kQuickActionsTabMotionCurve,
                builder: (context, hiddenProgress, child) => Opacity(
                  key: const Key('channel-quick-actions-opacity'),
                  opacity: 1 - hiddenProgress,
                  child: Transform.translate(
                    key: const Key('channel-quick-actions-transform'),
                    offset: Offset(
                      placement.hiddenHorizontalOffset * hiddenProgress,
                      0,
                    ),
                    child: Transform.scale(
                      key: const Key('channel-quick-actions-scale'),
                      scale:
                          1 -
                          ((1 - _kQuickActionsHiddenScale) * hiddenProgress),
                      child: child,
                    ),
                  ),
                ),
                child: _MorphingQuickActionsButton(
                  open: effectiveOpen,
                  openEdgeOffset: placement.rightInset - Grid.gutter,
                  onToggle: () =>
                      quickActionsOpen.value = !quickActionsOpen.value,
                  onSelected: (action) => unawaited(selectQuickAction(action)),
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

BoxConstraints _quickActionSheetConstraints(BuildContext context) {
  final mediaQuery = MediaQuery.of(context);
  return BoxConstraints(
    maxHeight: mediaQuery.size.height - mediaQuery.viewPadding.top - Grid.sm,
  );
}
