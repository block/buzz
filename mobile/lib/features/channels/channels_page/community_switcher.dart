part of '../channels_page.dart';

const _communityGridAvatarSize = 88.0;
const _communityCenterDuration = Duration(milliseconds: 420);
const _communityCenterCurve = Cubic(0.22, 1, 0.36, 1);
const _communityDepartureDuration = Duration(milliseconds: 520);
const _communityDepartureCurve = Cubic(0.2, 0, 0.2, 1);
const _communityRevealDuration = Duration(milliseconds: 160);
const _communityCenteredAvatarSize = _communityGridAvatarSize * 1.5;

// Readiness follows the data that determines the landing page's layout. Live
// messages continue to sync normally after this first usable snapshot.
final _communityContentReadyProvider = Provider.autoDispose<AsyncValue<bool>>((
  ref,
) {
  final channels = ref.watch(channelsProvider);
  final profile = ref.watch(profileProvider);
  final community = ref.watch(activeCommunityProvider).unwrapPrevious().value;
  final sections = ref.watch(channelSectionsProvider).isReady;
  final stars = ref.watch(channelStarsProvider).isReady;
  final sort = ref.watch(channelSortProvider).isReady;
  final mutes = ref.watch(channelMutesProvider).isReady;
  final readState = ref.watch(readStateProvider).isReady;
  final hasIdentity = ref.watch(myPubkeyProvider) != null;
  if (!channels.isLoading && channels.hasError) {
    return AsyncError(channels.error!, channels.stackTrace!);
  }
  return AsyncData(
    community != null &&
        channels.hasValue &&
        !channels.isLoading &&
        !profile.isLoading &&
        (!hasIdentity || (sections && stars && sort && mutes && readState)),
  );
});

/// Reuses Settings' entrance/dismissal, while letting a selected avatar leave
/// independently of the surface. The flight already performs the exit fade.
class _CommunitySwitcherRoute extends _SettingsPageRoute {
  _CommunitySwitcherRoute({
    required super.builder,
    required super.onTransitionProgress,
  }) : super(opaque: false);

  bool flying = false;

  @override
  bool didPop(void result) {
    if (flying) controller?.reverseDuration = Duration.zero;
    return super.didPop(result);
  }

  // Keep the entrance wrappers mounted during selection. Replacing them with
  // the bare page would reparent the native header as the center motion starts.
}

class _CommunityFlight {
  const _CommunityFlight(this.community, this.origin, this.previousCommunityId);

  final Community community;
  final Rect origin;
  final String? previousCommunityId;
}

class _CommunitySwitcherPage extends HookConsumerWidget {
  const _CommunitySwitcherPage({
    required this.destination,
    this.arrivingCommunity,
    required this.prepareLanding,
    required this.onFlightChanged,
    required this.onTransitionProgress,
  });

  final Rect? destination;
  final Community? arrivingCommunity;
  final Future<Rect?> Function() prepareLanding;
  final ValueChanged<bool> onFlightChanged;
  final ValueChanged<double> onTransitionProgress;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communitiesAsync = ref.watch(communityListProvider);
    final activeId = ref.watch(activeCommunityProvider).value?.id;
    final isEditing = useState(false);
    final flight = useState<_CommunityFlight?>(
      arrivingCommunity == null
          ? null
          : _CommunityFlight(
              arrivingCommunity!,
              Rect.fromCenter(
                center: MediaQuery.sizeOf(context).center(Offset.zero),
                width: _communityCenteredAvatarSize,
                height: _communityCenteredAvatarSize,
              ),
              null,
            ),
    );
    final landingBounds = useState(destination);
    if (flight.value != null) ref.watch(_communityContentReadyProvider);
    final error = useState<String?>(null);
    final failedCommunityId = useState<String?>(null);
    final centering = useAnimationController(
      duration: _communityCenterDuration,
      initialValue: arrivingCommunity == null ? 0 : 1,
    );
    final controller = useAnimationController(
      duration: _communityDepartureDuration,
    );
    final reveal = useAnimationController(duration: _communityRevealDuration);
    final flightAvatar = useMemoized(() {
      final selected = flight.value;
      return selected == null
          ? const SizedBox.shrink()
          : RepaintBoundary(
              // Keep avatar decoding/painting out of the per-frame motion build.
              child: Material(
                type: MaterialType.transparency,
                child: CommunityAvatar(
                  name: selected.community.name,
                  relayUrl: selected.community.relayUrl,
                  size: _communityGridAvatarSize,
                ),
              ),
            );
    }, [flight.value]);
    final reducedMotion =
        MediaQuery.disableAnimationsOf(context) ||
        FocusManager.instance.highlightMode == FocusHighlightMode.traditional;

    Future<void> waitForContent() async {
      final ready = Completer<void>();
      final subscription = ref.listenManual(_communityContentReadyProvider, (
        _,
        next,
      ) {
        if (ready.isCompleted) return;
        if (next.hasError) {
          ready.completeError(next.error!, next.stackTrace);
        } else if (next.value == true) {
          ready.complete();
        }
      }, fireImmediately: true);
      try {
        // A stalled relay must not trap someone behind an endless spinner.
        await ready.future.timeout(const Duration(seconds: 20));
      } finally {
        subscription.close();
      }
    }

    Future<void> selectCommunity(
      Community community,
      Rect origin, {
      bool arriving = false,
    }) async {
      if (flight.value != null && !arriving) return;
      if (!arriving &&
          community.id == activeId &&
          community.id != failedCommunityId.value) {
        Navigator.of(context).pop();
        return;
      }
      error.value = null;
      flight.value = _CommunityFlight(community, origin, activeId);
      onFlightChanged(true);
      if (!arriving) unawaited(HapticFeedback.selectionClick());
      final animate = !reducedMotion && (arriving || destination != null);
      try {
        // Relay teardown, credential changes and provider hydration must not
        // compete with the grid-to-center motion on the UI isolate.
        if (animate && !arriving) {
          await centering.forward(from: 0).orCancel;
        } else {
          centering.value = 1;
        }
        if (!context.mounted) return;
        await WidgetsBinding.instance.endOfFrame;
        if (!context.mounted) return;
        if (!arriving) {
          await ref
              .read(communityListProvider.notifier)
              .switchCommunity(community.id);
        }
        if (!context.mounted) return;
        await ref.read(activeCommunityProvider.future);
        if (!context.mounted) return;
        // Refresh this scope before awaiting its content. ChannelsNotifier
        // fences cached snapshots to the destination relay and identity.
        if (!arriving) ref.invalidate(channelsProvider);
        await (() async {
          await waitForContent();
          if (!context.mounted) return;
          final bounds = await prepareLanding();
          if (context.mounted) landingBounds.value = bounds;
        })().timeout(const Duration(seconds: 20));
        if (!context.mounted) return;
        if (animate) {
          await controller.forward(from: 0).orCancel;
          if (!context.mounted) return;
          // Reveal the settled page only after the avatar stops moving.
          await reveal.forward(from: 0).orCancel;
        } else {
          controller.value = 1;
          reveal.value = 1;
        }
        if (!context.mounted) return;
        Navigator.of(context).pop();
      } catch (e) {
        if (!context.mounted) return;
        failedCommunityId.value = community.id;
        flight.value = null;
        centering.reset();
        controller.reset();
        reveal.reset();
        onFlightChanged(false);
        onTransitionProgress(1);
        error.value = e is TimeoutException
            ? 'This community is taking longer to load. Select it to try again, or choose another.'
            : 'Could not switch communities. Please try again.';
      }
    }

    useEffect(() {
      if (arrivingCommunity == null) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        unawaited(
          selectCommunity(
            arrivingCommunity!,
            flight.value!.origin,
            arriving: true,
          ),
        );
      });
      return null;
    }, const []);

    Future<void> addCommunity() async {
      final navigator = Navigator.of(context, rootNavigator: true);
      final route = ModalRoute.of(context);
      final pairing = ref.read(pairingProvider.notifier);
      Navigator.of(context).pop();
      // Finish the picker exit before showing the pairing page.
      if (route != null) await route.completed;
      if (!navigator.mounted) return;
      pairing.reset();
      unawaited(
        navigator.push(
          MaterialPageRoute<void>(
            builder: (_) => const PairingPage(addingCommunity: true),
          ),
        ),
      );
    }

    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final header = ios
        ? SizedBox(
            height: frostedAppBarHeight(context),
            child: IosNavigationBar(
              title: '',
              leading: IosNavigationAction(
                label: 'Close community switcher',
                symbol: 'xmark',
                onPressed: () => Navigator.of(context).pop(),
              ),
              actions: [
                IosNavigationAction(
                  label: isEditing.value ? 'Done' : 'Edit',
                  onPressed: () => isEditing.value = !isEditing.value,
                ),
              ],
            ),
          )
        : Padding(
            padding: const EdgeInsets.all(Grid.xxs),
            child: Row(
              children: [
                IconButton(
                  tooltip: 'Close community switcher',
                  onPressed: () => Navigator.of(context).pop(),
                  icon: const Icon(BuzzIcons.x),
                ),
                const Spacer(),
                TextButton(
                  key: const Key('community-switcher-edit'),
                  onPressed: () => isEditing.value = !isEditing.value,
                  child: Text(isEditing.value ? 'Done' : 'Edit'),
                ),
              ],
            ),
          );
    final body = SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xs,
        Grid.gutter,
        Grid.md,
      ),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 440),
          child: Column(
            children: [
              Text(
                'Switch Community',
                key: const Key('community-switcher-title'),
                textAlign: TextAlign.center,
                style: context.textTheme.titleLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(height: Grid.md),
              if (error.value != null)
                Padding(
                  padding: const EdgeInsets.only(bottom: Grid.xs),
                  child: Semantics(
                    liveRegion: true,
                    child: Text(
                      error.value!,
                      style: TextStyle(color: context.colors.error),
                    ),
                  ),
                ),
              communitiesAsync.when(
                loading: () => const Center(
                  child: BuzzLoadingIndicator(
                    size: 40,
                    semanticLabel: 'Loading communities',
                  ),
                ),
                error: (e, _) => Text('Error loading communities: $e'),
                data: (communities) => _CommunityGrid(
                  children: [
                    for (final community in communities)
                      _CommunityGridTile(
                        community: community,
                        // Keep the current-community marker stable
                        // while the selected avatar leaves the grid.
                        isActive:
                            community.id ==
                            (flight.value == null
                                ? activeId
                                : flight.value!.previousCommunityId),
                        isEditing: isEditing.value,
                        hidden: flight.value?.community.id == community.id,
                        onSelect: selectCommunity,
                        onRemove: () => _confirmRemoveCommunity(
                          context,
                          ref,
                          community,
                          closeSheetAfterRemoval: community.id == activeId,
                        ),
                      ),
                    _CommunityGridAdd(onTap: addCommunity),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
    final surface = Material(
      key: const Key('community-switcher-page'),
      color: context.colors.surface,
      child: SafeArea(
        top: !ios,
        child: ios
            ? Stack(
                children: [
                  Positioned.fill(
                    top: frostedAppBarHeight(context),
                    child: body,
                  ),
                  // Paint the native bar last, as in FrostedScaffold. Painting
                  // Flutter content after a UIKit view can hide that content
                  // during this route's opacity/scale composition on iOS.
                  Positioned(top: 0, left: 0, right: 0, child: header),
                ],
              )
            : Column(
                children: [
                  header,
                  Expanded(child: body),
                ],
              ),
      ),
    );

    return PopScope(
      canPop: flight.value == null,
      child: IgnorePointer(
        ignoring: flight.value != null,
        child: AnimatedBuilder(
          animation: Listenable.merge([centering, controller, reveal]),
          child: RepaintBoundary(child: surface),
          builder: (context, child) {
            final selected = flight.value;
            final departure = _communityDepartureCurve.transform(
              controller.value,
            );
            final surfaceExit = Curves.easeOutCubic.transform(centering.value);
            final centerProgress = _communityCenterCurve.transform(
              centering.value,
            );
            final screen = MediaQuery.sizeOf(context);
            final center = Offset(screen.width / 2, screen.height / 2);
            final destinationRect = landingBounds.value;
            return Stack(
              fit: StackFit.expand,
              children: [
                if (selected != null)
                  Opacity(
                    key: const Key('community-loading-backdrop'),
                    opacity: 1 - Curves.easeOutCubic.transform(reveal.value),
                    child: ColoredBox(color: context.colors.surface),
                  ),
                Opacity(
                  key: const Key('community-picker-opacity'),
                  opacity: 1 - surfaceExit,
                  child: Transform.scale(
                    scale: 1 + 0.04 * surfaceExit,
                    child: child,
                  ),
                ),
                if (selected != null && controller.value == 0)
                  Positioned(
                    top: center.dy + _communityCenteredAvatarSize / 2 + Grid.md,
                    left: 0,
                    right: 0,
                    child: Center(
                      child: RepaintBoundary(
                        child: BuzzLoadingIndicator(
                          key: const Key('community-switch-loading'),
                          size: 18,
                          color: context.colors.onSecondaryContainer,
                          semanticLabel: 'Loading ${selected.community.name}',
                        ),
                      ),
                    ),
                  ),
                if (selected != null)
                  _CommunityFlyingAvatar(
                    avatar: flightAvatar,
                    origin: selected.origin.center,
                    destination:
                        destinationRect ??
                        Rect.fromCenter(
                          center: center,
                          width: _communityCenteredAvatarSize,
                          height: _communityCenteredAvatarSize,
                        ),
                    centerProgress: centerProgress,
                    departure: departure,
                  ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class _CommunityFlyingAvatar extends StatelessWidget {
  const _CommunityFlyingAvatar({
    required this.avatar,
    required this.origin,
    required this.destination,
    required this.departure,
    required this.centerProgress,
  });

  final Widget avatar;
  final Offset origin;
  final Rect destination;
  final double departure;
  final double centerProgress;

  @override
  Widget build(BuildContext context) {
    final screen = MediaQuery.sizeOf(context);
    final center = screen.center(Offset.zero);
    final toCenter = center - origin;
    final centerControl =
        (origin + center) / 2 + Offset(-toCenter.dy, toCenter.dx) * 0.18;
    final c = centerProgress;
    final start =
        origin * ((1 - c) * (1 - c)) +
        centerControl * (2 * (1 - c) * c) +
        center * (c * c);
    final end = destination.center;
    final delta = end - start;
    // A quadratic Bezier bends perpendicular to travel, like Motion's arc.
    // Bow outward first, then settle into the header. Clamp the control point
    // so the larger avatar stays inside narrow screens throughout the swoop.
    final bend = (start + end) / 2 + Offset(-delta.dy, delta.dx) * 0.75;
    final margin = _communityCenteredAvatarSize / 2 + Grid.xs;
    final width = screen.width;
    final control = Offset(
      bend.dx.clamp(margin, max(margin, width - margin)),
      bend.dy,
    );
    final t = departure;
    final position =
        start * ((1 - t) * (1 - t)) +
        control * (2 * (1 - t) * t) +
        end * (t * t);
    final centeredSize = lerpDouble(
      _communityGridAvatarSize,
      _communityCenteredAvatarSize,
      centerProgress,
    )!;
    final size = lerpDouble(centeredSize, destination.width, t)!;
    return Positioned(
      left: 0,
      top: 0,
      width: _communityGridAvatarSize,
      height: _communityGridAvatarSize,
      child: Transform.translate(
        offset:
            position -
            const Offset(
              _communityGridAvatarSize / 2,
              _communityGridAvatarSize / 2,
            ),
        child: ExcludeSemantics(
          child: Transform.scale(
            key: const Key('community-flying-avatar'),
            scale: size / _communityGridAvatarSize,
            child: avatar,
          ),
        ),
      ),
    );
  }
}

/// Intrinsic-height rows keep two columns without clipping scaled labels.
class _CommunityGrid extends StatelessWidget {
  const _CommunityGrid({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Column(
    key: const Key('community-switcher-options'),
    mainAxisSize: MainAxisSize.min,
    children: [
      for (var index = 0; index < children.length; index += 2)
        Padding(
          padding: const EdgeInsets.only(bottom: Grid.md),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: children[index]),
              const SizedBox(width: Grid.xs),
              Expanded(
                child: index + 1 < children.length
                    ? children[index + 1]
                    : const SizedBox.shrink(),
              ),
            ],
          ),
        ),
    ],
  );
}

class _CommunityGridTile extends HookWidget {
  const _CommunityGridTile({
    required this.community,
    required this.isActive,
    required this.isEditing,
    required this.hidden,
    required this.onSelect,
    required this.onRemove,
  });

  final Community community;
  final bool isActive;
  final bool isEditing;
  final bool hidden;
  final Future<void> Function(Community, Rect) onSelect;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final avatarKey = useMemoized(GlobalKey.new);
    return Semantics(
      selected: isActive,
      child: InkWell(
        key: Key('community-switcher-row-${community.id}'),
        borderRadius: BorderRadius.circular(Radii.card),
        onTap: isEditing
            ? null
            : () {
                final box = avatarKey.currentContext?.findRenderObject();
                if (box is RenderBox) {
                  unawaited(
                    onSelect(
                      community,
                      box.localToGlobal(Offset.zero) & box.size,
                    ),
                  );
                }
              },
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
          child: Column(
            children: [
              Stack(
                clipBehavior: Clip.none,
                children: [
                  Opacity(
                    opacity: hidden ? 0 : 1,
                    child: SizedBox(
                      key: avatarKey,
                      child: CommunityAvatar(
                        key: Key('community-switcher-avatar-${community.id}'),
                        name: community.name,
                        relayUrl: community.relayUrl,
                        size: _communityGridAvatarSize,
                      ),
                    ),
                  ),
                  Positioned(
                    right: -Grid.twelve,
                    bottom: -Grid.twelve,
                    child: _CommunityGridAction(
                      community: community,
                      isActive: isActive,
                      isEditing: isEditing,
                      onRemove: onRemove,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: Grid.xs),
              Text(
                community.name,
                textAlign: TextAlign.center,
                style: context.textTheme.bodyLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _CommunityGridAdd extends StatelessWidget {
  const _CommunityGridAdd({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => InkWell(
    key: const Key('community-switcher-add'),
    borderRadius: BorderRadius.circular(Radii.card),
    onTap: onTap,
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      child: Column(
        children: [
          ClipRSuperellipse(
            borderRadius: BorderRadius.circular(
              Radii.card * _communityGridAvatarSize / 40,
            ),
            child: Container(
              width: _communityGridAvatarSize,
              height: _communityGridAvatarSize,
              color: context.colors.primaryContainer.withValues(alpha: 0.5),
              child: Icon(
                BuzzIcons.plus,
                size: 32,
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
          const SizedBox(height: Grid.xs),
          Text(
            'Add Community',
            textAlign: TextAlign.center,
            style: context.textTheme.bodyLarge?.copyWith(
              fontWeight: FontWeight.w500,
            ),
          ),
        ],
      ),
    ),
  );
}
