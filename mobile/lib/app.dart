import 'features/activity/activity_provider.dart';
import 'features/channels/default_agent_launcher.dart';
import 'features/agents/agents_page.dart';

import 'dart:async';

import 'hosted_app_gate.dart';
import 'onboarding_app_entry.dart';
import 'shared/widgets/ios_navigation_metrics.dart';
import 'shared/community/paired_community_landing.dart';
import 'shared/community/community_loading_surface.dart';

import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/age_gate/age_restriction_page.dart';
import 'features/age_gate/age_signal_provider.dart';
import 'features/channels/unread_badge/unread_badge_provider.dart';
import 'features/home/home_page.dart';
import 'features/channels/me/me_page.dart';
import 'features/channels/me/me_unread_provider.dart';
import 'features/invites/invite_create_page.dart';
import 'features/pairing/pairing_page.dart';
import 'features/pairing/pairing_provider.dart';
import 'features/channels/agent_activity/observer_subscription.dart';
import 'features/channels/channel_detail_page.dart';
import 'shared/widgets/app_route_observer.dart';
import 'features/profile/user_status_cache_provider.dart';
import 'features/profile/settings_profile_header.dart';
import 'features/profile/set_status_sheet.dart';
import 'features/profile/user_status_provider.dart';
import 'features/profile/profile_edit_page.dart';
import 'features/profile/profile_text_editor.dart';
import 'features/settings/settings_page.dart';
import 'features/settings/theme_picker_page.dart';
import 'shared/auth/auth.dart';
import 'shared/deeplink/pending_deep_link_provider.dart';
import 'shared/emoji/emoji_burst.dart';
import 'shared/relay/relay.dart';
import 'shared/theme/theme.dart';
import 'shared/widgets/buzz_loading_indicator.dart';
import 'shared/widgets/buzz_toast_host.dart';

final _mobileRootNavigatorKey = GlobalKey<NavigatorState>();

class App extends HookConsumerWidget {
  const App({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ageSignalState = ref.watch(ageSignalProvider);
    ref.listen(ageSignalProvider, (_, next) {
      if (next == AgeSignalState.restricted) {
        ref.read(pairingProvider.notifier).reset();
      }
    });
    final communityTheme = ageSignalState != AgeSignalState.restricted
        ? ref.watch(communityThemeProvider)
        : defaultCommunityTheme;
    final themeMode = communityTheme.mode;
    final authState = ref.watch(authProvider);
    final pairedCommunity = ref.watch(pairedCommunityLandingProvider);

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        unawaited(ref.read(ageSignalProvider.notifier).request());
      });
      return null;
    }, const []);

    // Eagerly initialize websocket session and lifecycle observer when
    // authenticated. These providers connect and manage the websocket.
    if (ageSignalState != AgeSignalState.restricted &&
        authState.value?.status == AuthStatus.authenticated) {
      ref.watch(relaySessionProvider);
      // Listen, not watch: these publish often, and an App rebuild rebuilds
      // every route, including an open chat.
      ref.listen(observerRelayProvider, (_, _) {});
      ref.watch(appLifecycleProvider);
      ref.listen(userStatusCacheProvider, (_, _) {});
      // Activity owns hidden-DM recovery even when Inbox is never opened.
      ref.listen(activityProvider, (_, _) {});
      // Preserve initial read seeding even while Solo is hidden.
      ref.listen(meUnreadChannelIdsProvider, (_, _) {});
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
      if (ageSignalState != AgeSignalState.restricted) {
        applyBadge(ref.read(unreadBadgeProvider));
      } else {
        AppBadgePlus.updateBadge(0);
      }
      return null;
    }, [ageSignalState]);
    if (ageSignalState != AgeSignalState.restricted) {
      ref.listen<UnreadBadgeState>(unreadBadgeProvider, (_, next) {
        applyBadge(next);
      });
    }

    return MaterialApp(
      navigatorKey: _mobileRootNavigatorKey,
      navigatorObservers: [appRouteObserver],
      title: 'Buzz',
      debugShowCheckedModeBanner: false,
      theme: mobileAppearanceTheme(Brightness.light, communityTheme.accent),
      darkTheme: mobileAppearanceTheme(Brightness.dark, communityTheme.accent),
      themeMode: themeMode,
      // Above the navigator, so an age restriction cannot be bypassed by a
      // route that was pushed while the store signal request was in flight.
      builder: (context, child) => switch (ageSignalState) {
        AgeSignalState.restricted => const AgeRestrictionPage(),
        _ => BuzzToastHost(
          child: IosNavigationMetricsHost(
            child: AppMarkdownTheme(
              child: MobileHuddleShell(
                navigatorKey: _mobileRootNavigatorKey,
                child: EmojiBurstOverlay(
                  child: Stack(
                    fit: StackFit.expand,
                    children: [
                      ExcludeSemantics(
                        excluding: pairedCommunity != null,
                        child: AbsorbPointer(
                          absorbing: pairedCommunity != null,
                          child: child ?? const SizedBox.shrink(),
                        ),
                      ),
                      // Cover the navigator while onboarding is removed and the
                      // destination installs the matching avatar-flight route.
                      if (pairedCommunity != null)
                        CommunityLoadingSurface(
                          name: pairedCommunity.name,
                          relayUrl: pairedCommunity.relayUrl,
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      },
      home: authState.when(
        loading: () => const _SplashScreen(),
        error: (_, _) => const PairingPage(),
        data: (state) => switch (state.status) {
          AuthStatus.authenticated => HostedAppGate(
            authenticated: true,
            child: HomePage(
              onAgents: showAgents,
              defaultAgentBuilder: (context) => DefaultAgentLauncher(
                onChooseAgent: () => showAgents(context, createOnOpen: true),
              ),
              mePageBuilder: (_, chat, workspace) => MePage(
                sliver: true,
                newChatAction: chat,
                newWorkspaceAction: workspace,
              ),
              settingsPageBuilder: _buildSettingsPage,
              communityInvitePageBuilder: (_) => const CommunityInvitePage(),
              communityAppearancePageBuilder: (_) => const ThemePickerPage(),
            ),
          ),
          _ => const HostedAppGate(
            authenticated: false,
            child: OnboardingAppEntry(),
          ),
        },
      ),
    );
  }
}

Widget _buildSettingsPage(BuildContext context) => const _SettingsPageContent();

class _SettingsPageContent extends ConsumerWidget {
  const _SettingsPageContent();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return SettingsPage(
      profileHeader: const SettingsProfileHeader(),
      profileEditPageBuilder: (_) =>
          const ProfileEditPage(startInPhotoEditor: true),
      onSetStatus: (context) => showSetStatusSheet(
        context,
        currentStatus: ref.read(userStatusProvider).asData?.value,
      ),
      onEditDisplayName: showProfileDisplayNameEditor,
      onEditProfileDescription: showProfileDescriptionEditor,
      identityRecoveryPageBuilder: (_) =>
          const PairingPage(addingCommunity: true, identityRecoveryOnly: true),
    );
  }
}

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
