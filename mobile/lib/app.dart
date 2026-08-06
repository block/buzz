import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/material.dart';
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
          AuthStatus.authenticated => DeepLinkDispatcher(
            child: HomePage(
              settingsPageBuilder: _buildSettingsPage,
              hasUnreadInbox: hasUnreadInbox,
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

Widget _buildSettingsPage(BuildContext context) =>
    const SettingsPage(profileHeader: SettingsProfileHeader());

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
