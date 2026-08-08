part of '../channels_page.dart';

const _kQuickActionsTabMotionDuration = Duration(milliseconds: 220);
const _kQuickActionsTabMotionCurve = Cubic(0.77, 0, 0.175, 1);
const _kQuickActionsHiddenOverlap = Grid.half;
const _kQuickActionsHiddenScale = 0.8;

/// Places the channel quick-actions button beside mobile navigation.
///
/// The button remains available only on Home and moves behind the navigation
/// bar when another destination is selected.
class ChannelQuickActionsLauncher extends HookConsumerWidget {
  /// Whether the launcher should be visible beside the navigation bar.
  final bool visible;

  /// Height of the navigation bar used to vertically center the closed button.
  final double navigationBarHeight;

  /// Space between the navigation bar and the bottom safe area.
  final double navigationBarBottomGap;

  /// Full width of the navigation bar, including its inner edge padding.
  final double navigationBarWidth;

  /// Bottom system inset used by the navigation bar's [SafeArea].
  final double systemBottomInset;

  /// Distance between the launcher and the right edge of the screen.
  final double rightInset;

  /// Creates a launcher aligned with the supplied navigation geometry.
  const ChannelQuickActionsLauncher({
    super.key,
    required this.visible,
    required this.navigationBarHeight,
    required this.navigationBarBottomGap,
    required this.navigationBarWidth,
    required this.systemBottomInset,
    required this.rightInset,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final quickActionsOpen = useState(false);
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final navigationBottomInset = systemBottomInset > navigationBarBottomGap
        ? systemBottomInset
        : navigationBarBottomGap;
    final closedBottomInset =
        navigationBottomInset + ((navigationBarHeight - _kMorphClosedSize) / 2);
    final openLift =
        navigationBarHeight +
        Grid.xxs -
        ((navigationBarHeight - _kMorphClosedSize) / 2);
    final effectiveOpen = visible && quickActionsOpen.value;
    final screenWidth = MediaQuery.sizeOf(context).width;
    final navigationBarRight = (screenWidth + navigationBarWidth) / 2;
    final launcherRight = screenWidth - rightInset;
    final hiddenHorizontalOffset =
        navigationBarRight - launcherRight - _kQuickActionsHiddenOverlap;

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
          right: rightInset,
          bottom: closedBottomInset + (effectiveOpen ? openLift : 0),
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
                    offset: Offset(hiddenHorizontalOffset * hiddenProgress, 0),
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
                  openEdgeOffset: rightInset - Grid.gutter,
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
