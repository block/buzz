import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/activity/activity_provider.dart';
import 'features/activity/inbox_local_state_provider.dart';
import 'features/activity/inbox_read_state.dart';
import 'features/channels/unread_badge/unread_badge_provider.dart';
import 'features/home/home_page.dart';
import 'features/pairing/pairing_page.dart';
import 'features/channels/agent_activity/observer_subscription.dart';
import 'features/channels/deep_link_dispatcher.dart';
import 'features/profile/user_status_cache_provider.dart';
import 'features/profile/settings_profile_header.dart';
import 'features/settings/settings_page.dart';
import 'shared/auth/auth.dart';
import 'shared/deeplink/pending_deep_link_provider.dart';
import 'shared/emoji/emoji_burst.dart';
import 'shared/relay/relay.dart';
import 'shared/read_state/read_state_provider.dart';
import 'shared/security/sensitive_action_authorizer.dart';
import 'shared/theme/theme.dart';
import 'shared/widgets/buzz_loading_indicator.dart';

/// App-shell projection that joins Activity state for the Home navigation.
///
/// This belongs at the composition root because it deliberately aggregates
/// Activity feature providers for a sibling navigation surface.
final _unreadInboxItemCountProvider = Provider<int>((ref) {
  final readState = ref.watch(readStateProvider);
  if (!readState.isReady) return 0;

  final localState = ref.watch(inboxLocalStateProvider);
  final items = ref.watch(inboxItemsProvider);
  return items
      .where(
        (item) => !isInboxItemDone(
          item,
          markerOf: readState.effectiveTimestamp,
          localUnreadOverrides: localState.unreadIds,
          localDoneSet: localState.doneIds,
        ),
      )
      .length;
});

class App extends HookConsumerWidget {
  const App({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communityTheme = ref.watch(communityThemeProvider);
    final themeMode = communityTheme.mode;
    final accentIndex = effectiveAccentIndex(
      communityTheme.theme,
      communityTheme.accent,
    );
    final schemeName = communityTheme.theme;
    final authState = ref.watch(authProvider);

    final resolved = resolveSchemes(schemeName, themeMode);
    final lightScheme = applyAccent(resolved.light, accentIndex);
    final darkScheme = applyAccent(resolved.dark, accentIndex);
    // Light/Dark modes pin the brightness; System leaves it null so Flutter
    // follows the OS across the selected theme and its pair.
    final effectiveMode = resolved.forcedMode ?? themeMode;

    // Derive the gradient from the themes that produced each color scheme.
    // This keeps fallbacks and pinned brightness changes aligned with the
    // rendered palette rather than the raw persisted selection.
    final buzzLightGradient = buzzTopSectionGradient(
      resolved.lightTheme?.name ?? '',
      lightScheme.brightness,
    );
    final buzzDarkGradient = buzzTopSectionGradient(
      resolved.darkTheme?.name ?? '',
      darkScheme.brightness,
    );

    // Eagerly initialize websocket session and lifecycle observer when
    // authenticated. These providers connect and manage the websocket.
    var hasUnreadInbox = false;
    if (authState.value?.status == AuthStatus.authenticated) {
      ref.watch(relaySessionProvider);
      ref.watch(observerRelayProvider);
      ref.watch(appLifecycleProvider);
      ref.watch(userStatusCacheProvider);
      hasUnreadInbox = ref.watch(_unreadInboxItemCountProvider) > 0;
    }

    // Start listening for buzz:// links immediately (even pre-auth) so a
    // cold-start link survives until the authenticated UI can dispatch it.
    ref.watch(pendingDeepLinkProvider);

    void applyBadge(UnreadBadgeState state) {
      if (state.highPriorityCount > 0) {
        AppBadgePlus.updateBadge(state.highPriorityCount);
      } else if (state.generalUnreadCount > 0) {
        AppBadgePlus.updateBadge(1);
      } else {
        AppBadgePlus.updateBadge(0);
      }
    }

    useEffect(() {
      applyBadge(ref.read(unreadBadgeProvider));
      return null;
    }, const []);
    ref.listen<UnreadBadgeState>(unreadBadgeProvider, (_, next) {
      applyBadge(next);
    });

    return MaterialApp(
      title: 'Buzz',
      theme: AppTheme.light(
        colorScheme: lightScheme,
        topSectionGradient: buzzLightGradient,
      ),
      darkTheme: AppTheme.dark(
        colorScheme: darkScheme,
        topSectionGradient: buzzDarkGradient,
      ),
      themeMode: effectiveMode,
      // Above the navigator, so a burst keeps playing over a pushed thread page
      // or a modal sheet — the same reason desktop pins its canvas to the
      // viewport rather than to the message row.
      builder: (context, child) =>
          EmojiBurstOverlay(child: child ?? const SizedBox.shrink()),
      home: authState.when(
        loading: () => const _SplashScreen(),
        error: (_, _) => const PairingPage(),
        data: (state) => switch (state.status) {
          AuthStatus.authenticated => AppLockGate(
            communityId: state.community?.id,
            enabled:
                state.community?.sensitiveActionPolicy ==
                SensitiveActionPolicy.enabled,
            child: KeyedSubtree(
              key: ValueKey(state.community?.id),
              child: DeepLinkDispatcher(
                child: HomePage(
                  settingsPageBuilder: _buildSettingsPage,
                  hasUnreadInbox: hasUnreadInbox,
                ),
              ),
            ),
          ),
          _ => const DeepLinkDispatcher(
            dispatchMessageLinks: false,
            child: PairingPage(),
          ),
        },
      ),
    );
  }
}

Widget _buildSettingsPage(BuildContext context) => SettingsPage(
  profileHeader: const SettingsProfileHeader(),
  identityRecoveryPageBuilder: (_) =>
      const PairingPage(addingCommunity: true, identityRecoveryOnly: true),
);

class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(
      body: Center(
        child: BuzzLoadingIndicator(size: 56, semanticLabel: 'Starting Buzz'),
      ),
    );
  }
}

const appLockTimeout = Duration(minutes: 5);
const _appPrivacyChannel = MethodChannel('xyz.block.buzz/app_privacy');

class AppLockGate extends ConsumerStatefulWidget {
  const AppLockGate({
    super.key,
    required this.enabled,
    required this.child,
    this.communityId,
  });

  final bool enabled;
  final Widget child;
  final String? communityId;

  @override
  ConsumerState<AppLockGate> createState() => _AppLockGateState();
}

class _AppLockGateState extends ConsumerState<AppLockGate>
    with WidgetsBindingObserver {
  bool _locked = true;
  bool _authenticating = false;
  DateTime? _backgroundedAt;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _setAndroidRecentsProtection(widget.enabled);
    if (!widget.enabled) _locked = false;
    WidgetsBinding.instance.addPostFrameCallback((_) => _unlockIfNeeded());
  }

  @override
  void didUpdateWidget(AppLockGate oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.enabled != widget.enabled) {
      _setAndroidRecentsProtection(widget.enabled);
    }
    if (!widget.enabled) {
      setState(() => _locked = false);
    } else if (!oldWidget.enabled ||
        oldWidget.communityId != widget.communityId) {
      setState(() => _locked = true);
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => _unlockIfNeeded(forceFresh: true),
      );
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (!widget.enabled ||
        _authenticating ||
        ref.read(sensitiveActionAuthorizationSessionProvider).isAuthorizing) {
      return;
    }
    switch (state) {
      case AppLifecycleState.inactive:
      case AppLifecycleState.paused:
      case AppLifecycleState.hidden:
      case AppLifecycleState.detached:
        _backgroundedAt ??= ref.read(appLockClockProvider)();
        setState(() => _locked = true);
      case AppLifecycleState.resumed:
        final backgroundedAt = _backgroundedAt;
        _backgroundedAt = null;
        if (backgroundedAt != null) {
          final timedOut =
              ref.read(appLockClockProvider)().difference(backgroundedAt) >=
              appLockTimeout;
          _unlockIfNeeded(forceFresh: timedOut);
        }
    }
  }

  Future<void> _unlockIfNeeded({bool forceFresh = false}) async {
    if (!mounted || !widget.enabled || !_locked || _authenticating) return;
    final session = ref.read(sensitiveActionAuthorizationSessionProvider);
    if (!forceFresh && session.wasAuthorizedWithin(appLockTimeout)) {
      setState(() => _locked = false);
      return;
    }

    setState(() => _authenticating = true);
    final result = await session.authorize();
    if (!mounted || !widget.enabled) return;
    setState(() {
      _authenticating = false;
      _locked = result != DeviceAuthResult.success;
    });
  }

  Future<void> _setAndroidRecentsProtection(bool enabled) async {
    if (defaultTargetPlatform != TargetPlatform.android) return;
    try {
      await _appPrivacyChannel.invokeMethod<void>(
        'setRecentsProtection',
        enabled,
      );
    } on MissingPluginException {
      // Widget tests and unsupported embedders do not register this channel.
    }
  }

  @override
  void dispose() {
    _setAndroidRecentsProtection(false);
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  Future<void> _leaveCommunity() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Leave community?'),
        content: const Text(
          'This removes the community and its identity from this phone.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Leave community'),
          ),
        ],
      ),
    );
    if (confirmed == true && mounted) {
      await ref.read(authProvider.notifier).signOut();
    }
  }

  @override
  Widget build(BuildContext context) {
    final locked = widget.enabled && _locked;
    return Stack(
      fit: StackFit.expand,
      children: [
        Offstage(offstage: locked, child: widget.child),
        if (locked) _buildLockScreen(context),
      ],
    );
  }

  Widget _buildLockScreen(BuildContext context) {
    final authenticationName = sensitiveActionAuthenticationName(
      Theme.of(context).platform,
    );
    return Scaffold(
      key: const Key('app-lock-screen'),
      backgroundColor: Colors.black,
      body: SafeArea(
        child: Stack(
          fit: StackFit.expand,
          children: [
            Center(
              child: Padding(
                padding: const EdgeInsets.all(32),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Image.asset(
                      'assets/images/buzz-icon.png',
                      key: const Key('app-lock-logo'),
                      width: 112,
                      semanticLabel: 'Buzz',
                    ),
                    const SizedBox(height: 32),
                    FilledButton(
                      key: const Key('app-lock-unlock-button'),
                      style: FilledButton.styleFrom(
                        backgroundColor: Colors.white,
                        foregroundColor: Colors.black,
                        disabledBackgroundColor: Colors.white54,
                        disabledForegroundColor: Colors.black54,
                      ),
                      onPressed: _authenticating ? null : _unlockIfNeeded,
                      child: Text(
                        _authenticating
                            ? 'Authenticating…'
                            : 'Unlock with $authenticationName',
                      ),
                    ),
                  ],
                ),
              ),
            ),
            Align(
              alignment: Alignment.bottomCenter,
              child: Padding(
                padding: const EdgeInsets.only(bottom: 12),
                child: TextButton(
                  key: const Key('app-lock-leave-community'),
                  style: TextButton.styleFrom(
                    foregroundColor: Colors.white54,
                    textStyle: Theme.of(context).textTheme.bodySmall,
                  ),
                  onPressed: _leaveCommunity,
                  child: const Text('Leave community'),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
