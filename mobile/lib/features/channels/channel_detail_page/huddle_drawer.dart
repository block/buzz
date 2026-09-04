part of '../channel_detail_page.dart';

const _mobileHuddleDrawerBaseHeight = 80.0;
const _mobileHuddleDrawerRadius = 24.0;
const _mobileHuddleDrawerMotion = Duration(milliseconds: 260);
const _mobileHuddleDrawerCurve = Cubic(0.32, 0.72, 0, 1);

/// Lifts the mobile app above a persistent Huddle control drawer when the
/// full-screen call has been minimized.
class MobileHuddleShell extends HookConsumerWidget {
  const MobileHuddleShell({
    super.key,
    required this.navigatorKey,
    required this.child,
  });

  final GlobalKey<NavigatorState> navigatorKey;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    ref.read(mobileHuddleControllerProvider.notifier);
    final presentation = ref.watch(mobileHuddlePresentationProvider);
    final session = ref.watch(huddleSessionProvider);
    final relayStatus = ref.watch(relaySessionProvider).status;
    _useIncomingHuddleReactions(
      context: context,
      ref: ref,
      inSession: session.isInSession,
      channelId: session.ephemeralChannelId,
      currentPubkey: session.currentPubkey,
      relayStatus: relayStatus,
    );
    final drawerOpen =
        presentation == MobileHuddlePresentation.drawer && session.isInSession;
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final duration = reducedMotion ? Duration.zero : _mobileHuddleDrawerMotion;
    final drawerHeight =
        _mobileHuddleDrawerBaseHeight + MediaQuery.paddingOf(context).bottom;
    final surface = _huddleDrawerSurface(context);
    final usesNativeConcentricSurface =
        defaultTargetPlatform == TargetPlatform.iOS;

    return ColoredBox(
      color: drawerOpen ? surface : context.colors.surface,
      child: Stack(
        key: const ValueKey('mobile-huddle-shell'),
        fit: StackFit.expand,
        children: [
          AnimatedPositioned(
            key: const ValueKey('mobile-huddle-app-surface-position'),
            duration: duration,
            curve: _mobileHuddleDrawerCurve,
            top: 0,
            left: 0,
            right: 0,
            bottom: drawerOpen ? drawerHeight : 0,
            child: AnimatedContainer(
              key: const ValueKey('mobile-huddle-app-surface'),
              duration: duration,
              curve: _mobileHuddleDrawerCurve,
              clipBehavior: usesNativeConcentricSurface
                  ? Clip.none
                  : Clip.antiAlias,
              decoration: BoxDecoration(
                borderRadius: BorderRadius.vertical(
                  bottom: Radius.circular(
                    drawerOpen ? _mobileHuddleDrawerRadius : 0,
                  ),
                ),
                boxShadow: drawerOpen
                    ? [
                        BoxShadow(
                          color: context.colors.shadow.withValues(alpha: 0.44),
                          blurRadius: 24,
                          spreadRadius: -12,
                          offset: Offset(0, 10),
                        ),
                      ]
                    : const [],
              ),
              child: usesNativeConcentricSurface
                  ? ConcentricSheetSurface(
                      key: const ValueKey(
                        'mobile-huddle-app-concentric-surface',
                      ),
                      enabled: drawerOpen,
                      color: context.colors.surface,
                      backdropColor: surface,
                      corners: ConcentricSurfaceCorners.bottom,
                      padding: EdgeInsets.zero,
                      providesSheetSurface: false,
                      child: child,
                    )
                  : child,
            ),
          ),
          AnimatedPositioned(
            key: const ValueKey('mobile-huddle-drawer-position'),
            duration: duration,
            curve: _mobileHuddleDrawerCurve,
            left: 0,
            right: 0,
            bottom: drawerOpen ? 0 : -drawerHeight,
            height: drawerHeight,
            child:
                presentation != MobileHuddlePresentation.hidden ||
                    session.isInSession
                ? _MobileHuddleDrawer(
                    navigatorKey: navigatorKey,
                    session: session,
                  )
                : const SizedBox.shrink(),
          ),
        ],
      ),
    );
  }
}

class _MobileHuddleDrawer extends ConsumerWidget {
  const _MobileHuddleDrawer({
    required this.navigatorKey,
    required this.session,
  });

  final GlobalKey<NavigatorState> navigatorKey;
  final HuddleSessionState session;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final foreground = _huddleDrawerForeground(context);
    final controlSurface = _huddleDrawerControlSurface(context);
    final drawerSurface = _huddleDrawerSurface(context);
    final sessionController = ref.read(huddleSessionProvider.notifier);
    final lifecycleController = ref.read(
      mobileHuddleControllerProvider.notifier,
    );

    void restoreFullScreen() {
      if (ref.read(mobileHuddlePresentationProvider) !=
          MobileHuddlePresentation.drawer) {
        return;
      }
      final parentChannelId = session.parentChannelId;
      final ephemeralChannelId = session.ephemeralChannelId;
      final navigator = navigatorKey.currentState;
      final reducedMotion = MediaQuery.disableAnimationsOf(context);
      if (parentChannelId == null ||
          ephemeralChannelId == null ||
          navigator == null) {
        return;
      }
      ref.read(mobileHuddlePresentationProvider.notifier).showFullScreen();
      unawaited(
        Future<void>(() async {
          if (!reducedMotion) {
            await Future<void>.delayed(_mobileHuddleDrawerMotion);
          }
          if (!navigator.mounted ||
              !ref.read(huddleSessionProvider).isInSession ||
              ref.read(mobileHuddlePresentationProvider) !=
                  MobileHuddlePresentation.fullScreen) {
            return;
          }
          await navigator.push<void>(
            _mobileHuddleCallRoute(
              context: navigator.context,
              invite: _HuddleInvite(
                parentChannelId: parentChannelId,
                ephemeralChannelId: ephemeralChannelId,
                startedBy: session.isCreator ? session.currentPubkey ?? '' : '',
                startedEventId: session.startedEventId ?? '',
              ),
            ),
          );
        }),
      );
    }

    void leave() {
      ref.read(mobileHuddlePresentationProvider.notifier).hide();
      unawaited(
        lifecycleController.leave().catchError((Object error) {
          debugPrint('[MobileHuddle] drawer leave failed: $error');
        }),
      );
    }

    return Material(
      key: const ValueKey('mobile-huddle-drawer'),
      color: drawerSurface,
      child: InkWell(
        onTap: restoreFullScreen,
        child: Semantics(
          container: true,
          label: 'Huddle controls',
          hint: 'Tap outside a control to open the Huddle',
          child: Center(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Transform.translate(
                key: const ValueKey('huddle-drawer-control-offset'),
                offset: const Offset(0, -8),
                child: Row(
                  key: const ValueKey('huddle-drawer-controls'),
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    Row(
                      key: const ValueKey('huddle-drawer-primary-controls'),
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        _HuddleRoundControl(
                          key: const ValueKey('huddle-drawer-expand'),
                          tooltip: 'Open Huddle',
                          showTooltip: false,
                          icon: LucideIcons.chevronUp,
                          foregroundColor: foreground,
                          backgroundColor: controlSurface,
                          useHapticFeedback: true,
                          onPressed: restoreFullScreen,
                        ),
                        const SizedBox(width: Grid.twelve),
                        _HuddleRoundControl(
                          key: const ValueKey('huddle-drawer-speaker-toggle'),
                          tooltip: session.isSpeakerEnabled
                              ? 'Use earpiece'
                              : 'Use speaker',
                          showTooltip: false,
                          icon: LucideIcons.volume2,
                          foregroundColor: session.isSpeakerEnabled
                              ? drawerSurface
                              : foreground,
                          backgroundColor: session.isSpeakerEnabled
                              ? foreground
                              : controlSurface,
                          toggled: session.isSpeakerEnabled,
                          useHapticFeedback: true,
                          onPressed: () => unawaited(
                            sessionController.setSpeakerEnabled(
                              !session.isSpeakerEnabled,
                            ),
                          ),
                        ),
                        const SizedBox(width: Grid.twelve),
                        _HuddleRoundControl(
                          key: const ValueKey('huddle-drawer-mute-toggle'),
                          tooltip: session.isMuted ? 'Unmute' : 'Mute',
                          showTooltip: false,
                          icon: session.isMuted
                              ? LucideIcons.micOff
                              : LucideIcons.mic,
                          foregroundColor: session.isMuted
                              ? foreground
                              : drawerSurface,
                          backgroundColor: session.isMuted
                              ? controlSurface
                              : foreground,
                          toggled: session.isMuted,
                          useHapticFeedback: true,
                          onPressed: () => unawaited(
                            sessionController.setMuted(!session.isMuted),
                          ),
                        ),
                      ],
                    ),
                    _HuddleRoundControl(
                      key: const ValueKey('huddle-drawer-leave'),
                      tooltip: 'Leave Huddle',
                      showTooltip: false,
                      icon: LucideIcons.phoneOff,
                      foregroundColor: context.colors.error,
                      backgroundColor: controlSurface,
                      useHapticFeedback: true,
                      onPressed: leave,
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

Color _huddleDrawerSurface(BuildContext context) =>
    context.appColors.huddleDrawerSurface;

Color _huddleDrawerControlSurface(BuildContext context) =>
    context.appColors.huddleControlSurface;

Color _huddleDrawerForeground(BuildContext context) =>
    context.appColors.onHuddleDrawer;
