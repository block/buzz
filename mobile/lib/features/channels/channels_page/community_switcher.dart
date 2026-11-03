part of '../channels_page.dart';

const _communityGridAvatarSize = communityGridAvatarSize;
const _communityCenterDuration = Duration(milliseconds: 420);
const _communityCenterCurve = Cubic(0.22, 1, 0.36, 1);
const _communityDepartureDuration = Duration(milliseconds: 520);
const _communityDepartureCurve = Cubic(0.2, 0, 0.2, 1);
const _communityRevealDuration = Duration(milliseconds: 160);
const _communityCenteredAvatarSize = _communityGridAvatarSize * 1.5;

// Readiness follows the data that determines the landing page's layout. Live
// messages continue to sync normally after this first usable snapshot. The
// header's own profile avatar hydrates independently and never gates landing.
// Cached destinations use local layout/read preferences immediately. Remote
// changes may reorder the list or update badges after reveal; offline landing
// must not wait for those relay reads.
final _communityContentReadyProvider = Provider.autoDispose<AsyncValue<bool>>((
  ref,
) {
  final channels = ref.watch(channelsProvider);
  final community = ref.watch(activeCommunityProvider).unwrapPrevious().value;
  final sections = ref.watch(channelSectionsProvider).isReady;
  final stars = ref.watch(channelStarsProvider).isReady;
  final sort = ref.watch(channelSortProvider).isReady;
  final mutes = ref.watch(channelMutesProvider).isReady;
  final readState = ref.watch(readStateProvider).isReady;
  final hasIdentity = ref.watch(myPubkeyProvider) != null;
  // A restored snapshot lands with the local section/star/sort/mute/read state
  // already applied; their remote catch-up continues after reveal. Watching
  // channelsProvider above re-evaluates this when the snapshot is installed.
  final cached = ref.read(channelsProvider.notifier).hasCachedSnapshot;
  if (!channels.isLoading && channels.hasError) {
    return AsyncError(channels.error!, channels.stackTrace!);
  }
  return AsyncData(
    community != null &&
        channels.hasValue &&
        !channels.isLoading &&
        (!hasIdentity ||
            cached ||
            (sections && stars && sort && mutes && readState)),
  );
});

/// Uses a scale-and-fade entrance/dismissal, while letting a selected avatar leave
/// independently of the surface. The flight already performs the exit fade.
class _CommunitySwitcherRoute extends _CommunitySurfaceRoute {
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

/// Shared community management surface for app entry and switching.
class CommunitySwitcherPage extends HookConsumerWidget {
  const CommunitySwitcherPage({
    super.key,
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
        // Storage and relay teardown are asynchronous I/O, so they overlap the
        // grid-to-center motion. Both settle before any failure is handled, so
        // the picker never resets while one of them is still running.
        await Future.wait<void>([
          if (animate && !arriving)
            centering.forward(from: 0).orCancel
          else
            Future.sync(() => centering.value = 1),
          if (!arriving)
            ref
                .read(communityListProvider.notifier)
                .switchCommunity(community.id),
        ]);
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
            builder: (_) => const OnboardingAppEntry(addingCommunity: true),
          ),
        ),
      );
    }

    final header = FrostedAppBar(
      nativeTitle: '',
      automaticallyImplyLeading: false,
      nativeLeading: IosNavigationAction(
        label: 'Close community switcher',
        symbol: 'xmark',
        onPressed: () => Navigator.of(context).pop(),
      ),
      nativeActions: [
        IosNavigationAction(
          label: isEditing.value ? 'Done' : 'Edit',
          onPressed: () => isEditing.value = !isEditing.value,
        ),
      ],
      leading: IconButton(
        tooltip: 'Close community switcher',
        onPressed: () => Navigator.of(context).pop(),
        icon: const Icon(BuzzIcons.x),
      ),
      actions: [
        TextButton(
          key: const Key('community-switcher-edit'),
          onPressed: () => isEditing.value = !isEditing.value,
          child: Text(isEditing.value ? 'Done' : 'Edit'),
        ),
      ],
    );
    final body = SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        frostedAppBarHeight(context) + Grid.xs,
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
                data: (communities) => CommunityGrid(
                  children: [
                    for (final community in communities)
                      CommunityGridOption(
                        id: community.id,
                        name: community.name,
                        relayUrl: community.relayUrl,
                        // Keep the current-community marker stable
                        // while the selected avatar leaves the grid.
                        isActive:
                            community.id ==
                            (flight.value == null
                                ? activeId
                                : flight.value!.previousCommunityId),
                        enabled: !isEditing.value,
                        hidden: flight.value?.community.id == community.id,
                        onSelect: (origin) =>
                            selectCommunity(community, origin),
                        badge: _CommunityGridAction(
                          community: community,
                          isActive:
                              community.id ==
                              (flight.value == null
                                  ? activeId
                                  : flight.value!.previousCommunityId),
                          isEditing: isEditing.value,
                          onRemove: () => _confirmRemoveCommunity(
                            context,
                            ref,
                            community,
                            closeSheetAfterRemoval: community.id == activeId,
                          ),
                        ),
                      ),
                    CommunityGridAdd(onTap: addCommunity),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
    final surface = FrostedScaffold(
      key: const Key('community-switcher-page'),
      appBar: header,
      body: SafeArea(top: false, child: body),
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
    // Bow left first, then settle into the header. Clamp the control point
    // so the larger avatar stays inside narrow screens throughout the swoop.
    final bend = (start + end) / 2 + Offset(delta.dy, -delta.dx) * 0.75;
    final margin = _communityCenteredAvatarSize / 2 + Grid.xs;
    final width = screen.width;
    final control = Offset(
      bend.dx.clamp(margin, max(margin, width - margin)),
      bend.dy.clamp(margin, max(margin, screen.height - margin)),
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
