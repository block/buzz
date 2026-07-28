import 'dart:async';

import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/channels/unread_badge/unread_badge_provider.dart';
import 'features/home/home_page.dart';
import 'features/pairing/pairing_page.dart';
import 'features/channels/agent_activity/active_agent_turns.dart';
import 'features/channels/agent_activity/agent_live_updates.dart';
import 'features/channels/agent_activity/observer_subscription.dart';
import 'features/channels/channels_provider.dart';
import 'features/channels/deep_link_dispatcher.dart';
import 'features/profile/user_cache_provider.dart';
import 'features/profile/user_status_cache_provider.dart';
import 'features/profile/settings_profile_header.dart';
import 'features/settings/settings_page.dart';
import 'shared/auth/auth.dart';
import 'shared/deeplink/pending_deep_link_provider.dart';
import 'shared/notifications/agent_live_update_preferences.dart';
import 'shared/relay/relay.dart';
import 'shared/theme/theme.dart';
import 'shared/widgets/buzz_loading_indicator.dart';

class App extends HookConsumerWidget {
  const App({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final themeMode = ref.watch(themeProvider);
    final accentIndex = ref.watch(accentProvider);
    final schemeName = ref.watch(schemeProvider);
    final authState = ref.watch(authProvider);
    final isAuthenticated = authState.value?.status == AuthStatus.authenticated;
    final agentLiveUpdatesEnabled = ref.watch(agentLiveUpdatesEnabledProvider);
    final appLifecycleState = isAuthenticated
        ? ref.watch(appLifecycleProvider)
        : null;
    final activeAgentTurns = isAuthenticated
        ? ref.watch(activeAgentTurnsProvider)
        : const <ActiveAgentTurn>[];
    final agentLiveUpdate = isAuthenticated && agentLiveUpdatesEnabled
        ? _buildAgentLiveUpdate(ref, activeAgentTurns)
        : null;
    final agentLiveUpdateSynchronizer = useMemoized(
      AgentLiveUpdateSynchronizer.new,
    );
    final activeAgentPubkeys = activeAgentTurns
        .map((turn) => turn.agentPubkey)
        .toSet()
        .toList();

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
    if (isAuthenticated) {
      ref.watch(relaySessionProvider);
      ref.watch(observerRelayProvider);
      ref.watch(userStatusCacheProvider);
    }

    // Start listening for buzz:// links immediately (even pre-auth) so a
    // cold-start link survives until the authenticated UI can dispatch it.
    ref.watch(pendingDeepLinkProvider);

    useEffect(() {
      unawaited(agentLiveUpdateSynchronizer.sync(agentLiveUpdate));
      return null;
    }, [agentLiveUpdate, appLifecycleState]);

    useEffect(() {
      ref.read(userCacheProvider.notifier).preload(activeAgentPubkeys);
      return null;
    }, [activeAgentPubkeys.join('|')]);

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
      home: authState.when(
        loading: () => const _SplashScreen(),
        error: (_, _) => const PairingPage(),
        data: (state) => switch (state.status) {
          AuthStatus.authenticated => const DeepLinkDispatcher(
            child: HomePage(settingsPageBuilder: _buildSettingsPage),
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

AgentLiveUpdateContent? _buildAgentLiveUpdate(
  WidgetRef ref,
  List<ActiveAgentTurn> turns,
) {
  final channels = ref.watch(channelsProvider).value ?? const [];
  final channelNames = {
    for (final channel in channels) channel.id: channel.name,
  };
  final profiles = ref.watch(userCacheProvider);
  final agentNames = {
    for (final turn in turns)
      if (profiles[turn.agentPubkey.toLowerCase()] case final profile?)
        turn.agentPubkey: profile.label,
  };
  return buildAgentLiveUpdateContent(turns, channelNames, agentNames);
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
