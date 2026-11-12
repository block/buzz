import 'features/activity/activity_provider.dart';
import 'features/channels/bestie_conversation.dart';
import 'features/channels/default_agent_launcher.dart';
import 'features/agents/agents_page.dart';

import 'dart:async';

import 'hosted_app_gate.dart';
import 'onboarding_app_entry.dart';
import 'organization_app_entry.dart';
import 'shared/hosted/organization_environment.dart';
import 'shared/widgets/ios_navigation_metrics.dart';
import 'shared/community/paired_community_landing.dart';
import 'shared/community/community_loading_surface.dart';

import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import 'features/age_gate/age_restriction_page.dart';
import 'features/age_gate/age_signal_provider.dart';
import 'features/channels/channel.dart';
import 'features/channels/channel_management_provider.dart';
import 'features/channels/channels_provider.dart';
import 'features/channels/unread_badge/unread_badge_provider.dart';
import 'features/home/home_page.dart';
import 'features/channels/me/me_page.dart';
import 'features/channels/me/me_unread_provider.dart';
import 'features/invites/invite_create_page.dart';
import 'features/invites/invite_join_provider.dart';
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
import 'shared/widgets/covered_route_keyboard_insets.dart';

const _starterChannelNamespace = '3ce33bea-8f09-5f1b-9c85-8a7d2659e6b0';

const _starterChannels = [
  (slug: 'general', description: 'General conversation and community updates.'),
  (
    slug: 'welcome-everyone',
    description: 'Say hi, ask a question, or share what brought you here.',
  ),
];

/// Waits for this attempt's community to hold an authenticated connection.
///
/// Each attempt observes the live session, so a retry never reuses an earlier
/// attempt's failure or a connection that has since been lost.
Future<void> _waitForInviteRelay(Ref ref, bool Function() isScopeCurrent) {
  final connected = Completer<void>();
  final sub = ref.listen(relaySessionProvider, (_, next) {
    if (connected.isCompleted) return;
    if (!isScopeCurrent()) {
      connected.completeError(
        StateError('Active community changed during invite recovery'),
      );
    } else if (next.status == SessionStatus.connected) {
      connected.complete();
    }
  }, fireImmediately: true);
  return connected.future
      .timeout(const Duration(seconds: 15))
      .whenComplete(sub.close);
}

/// App-level bridge from invite joining to the channels feature.
class MobileInviteJoinRecovery implements InviteJoinRecovery {
  final Future<List<Channel>> Function() _loadChannels;
  final Future<Channel?> Function(String channelId) _fetchChannel;
  final Future<Channel> Function({
    required String channelId,
    required String name,
    required String channelType,
    required String visibility,
    String? description,
    int? ttlSeconds,
  })
  _createChannel;
  final Future<void> Function(String channelId) _joinChannel;
  final String _relayHttpOrigin;
  final bool Function() _isScopeCurrent;

  /// Creates recovery from channel-loading, single-channel lookup, creation,
  /// and join operations.
  MobileInviteJoinRecovery({
    required Future<List<Channel>> Function() loadChannels,
    required Future<Channel?> Function(String channelId) fetchChannel,
    required Future<Channel> Function({
      required String channelId,
      required String name,
      required String channelType,
      required String visibility,
      String? description,
      int? ttlSeconds,
    })
    createChannel,
    required Future<void> Function(String channelId) joinChannel,
    required String relayHttpOrigin,
    bool Function()? isScopeCurrent,
  }) : _loadChannels = loadChannels,
       _fetchChannel = fetchChannel,
       _createChannel = createChannel,
       _joinChannel = joinChannel,
       _relayHttpOrigin = relayHttpOrigin,
       _isScopeCurrent = isScopeCurrent ?? _alwaysCurrent;

  /// Ensures memberships in the same public starter channels as desktop.
  ///
  /// All starter work is fenced to the community and identity that started it.
  /// A partial setup remains recoverable, so a failed operation deliberately
  /// propagates to the invite flow instead of being reported as success. The
  /// one exception is a starter whose name or id the relay reports as taken
  /// by a channel the directory shows cannot serve as the starter, or an
  /// archived channel at the starter's own id: both
  /// are permanent, so the starter is skipped instead of blocking the join.
  @override
  Future<String?> ensureStarterChannels() async {
    _ensureScopeCurrent();
    var channels = await _loadChannels();
    _ensureScopeCurrent();
    String? welcomeEveryoneId;

    for (final starter in _starterChannels) {
      _ensureScopeCurrent();
      var channel = _findStarterChannel(channels, starter.slug);
      if (channel == null) {
        final channelId = desktopStarterChannelId(
          relayHttpOrigin: _relayHttpOrigin,
          slug: starter.slug,
        );
        try {
          channel = await _createChannel(
            channelId: channelId,
            name: starter.slug,
            channelType: 'stream',
            visibility: 'open',
            description: starter.description,
          );
          _ensureScopeCurrent();
        } catch (error) {
          final nameTaken = _isChannelNameTakenError(error);
          final archivedAtId = _isArchivedChannelError(error);
          if (!nameTaken && !archivedAtId && !_isDuplicateChannelError(error)) {
            rethrow;
          }
          _ensureScopeCurrent();
          channels = await _loadChannels();
          _ensureScopeCurrent();
          final holder =
              _findStarterChannel(channels, starter.slug) ??
              channels
                  .where((candidate) => candidate.id == channelId)
                  .firstOrNull ??
              (nameTaken ? _findListedNamesake(channels, starter.slug) : null);
          if (holder == null) {
            // The relay refuses a create at an archived channel's id before
            // its duplicate check, and keeps that id, so the starter can never
            // be created there.
            if (archivedAtId) continue;
            // The relay keeps the id (desktop uses the same one), so a
            // duplicate with no match in the reload may be an archived
            // starter whose metadata the reload did not return. Read that id
            // directly.
            if (!nameTaken && _isDuplicateChannelError(error)) {
              final atId = await _fetchChannel(channelId);
              _ensureScopeCurrent();
              if (atId != null && atId.isArchived) continue;
            }
            // A holder not visible yet may still be usable, so keep the setup
            // recoverable.
            rethrow;
          }
          // The relay keeps a channel's id and a listed channel's name, archived
          // ones included, so an unusable holder never frees the starter. Skip
          // it rather than strand the invite in permanent recovery.
          if (!_isUsableStarter(holder)) continue;
          channel = holder;
        }
      }

      _ensureScopeCurrent();
      if (!channel.isMember) {
        await _joinChannel(channel.id);
        _ensureScopeCurrent();
      }
      if (starter.slug == 'welcome-everyone') {
        welcomeEveryoneId = channel.id;
      }
    }

    return welcomeEveryoneId;
  }

  static bool _alwaysCurrent() => true;

  void _ensureScopeCurrent() {
    if (!_isScopeCurrent()) {
      throw StateError('Active community changed during invite recovery');
    }
  }
}

Channel? _findStarterChannel(List<Channel> channels, String name) => channels
    .where((channel) => _hasName(channel, name) && _isUsableStarter(channel))
    .firstOrNull;

/// The channel that holds [name] in the relay's listed-name rule: an open,
/// not unlisted stream or forum (`LISTED_CHANNEL_SQL` in buzz-db).
Channel? _findListedNamesake(List<Channel> channels, String name) => channels
    .where(
      (channel) =>
          _hasName(channel, name) &&
          channel.visibility == 'open' &&
          !channel.isUnlisted &&
          (channel.isStream || channel.isForum),
    )
    .firstOrNull;

bool _hasName(Channel channel, String name) =>
    channel.name.trim().toLowerCase() == name.trim().toLowerCase();

bool _isUsableStarter(Channel channel) =>
    channel.isStream && channel.visibility == 'open' && !channel.isArchived;

bool _isDuplicateChannelError(Object error) =>
    error.toString().contains('duplicate: channel already exists');

/// The relay refuses any channel event whose `h` is an archived channel,
/// a create at that id included: `invalid: channel is archived`.
bool _isArchivedChannelError(Object error) =>
    error.toString().contains('invalid: channel is archived');

/// The relay refuses creating an open channel whose name another listed
/// channel already holds, ignoring case:
/// `invalid: an open channel named "general" already exists`.
bool _isChannelNameTakenError(Object error) {
  final message = error.toString().toLowerCase();
  return message.contains('invalid: an open channel named') &&
      message.contains('already exists');
}

/// Returns desktop's deterministic per-relay UUID for a public starter.
@visibleForTesting
String desktopStarterChannelId({
  required String relayHttpOrigin,
  required String slug,
}) {
  final scope = relayHttpOrigin.trim().replaceFirst(RegExp(r'/+$'), '');
  return const Uuid().v5(
    _starterChannelNamespace,
    'starter-channel:v1:$scope:$slug',
  );
}

/// Builds one fresh, identity-scoped invite recovery against the app container.
InviteJoinRecovery buildMobileInviteJoinRecovery(
  Ref ref,
  InviteJoinRecoveryScope scope,
) {
  final expectedConfig = ref.read(relayConfigProvider);
  if (expectedConfig.baseUrl != scope.relayHttpOrigin ||
      expectedConfig.nsec != scope.nsec) {
    throw StateError('Active community changed before invite recovery');
  }
  final channelActions = ref.read(channelActionsProvider);

  bool isScopeCurrent() {
    final currentConfig = ref.read(relayConfigProvider);
    return currentConfig.baseUrl == scope.relayHttpOrigin &&
        currentConfig.nsec == scope.nsec;
  }

  void ensureScopeCurrent() {
    if (!isScopeCurrent()) {
      throw StateError('Active community changed during invite recovery');
    }
  }

  return MobileInviteJoinRecovery(
    loadChannels: () async {
      ensureScopeCurrent();
      await ref.read(activeCommunityProvider.future);
      ensureScopeCurrent();
      await _waitForInviteRelay(ref, isScopeCurrent);
      ensureScopeCurrent();
      final channels = await ref
          .read(channelsProvider.notifier)
          .fetchFresh(fetchDirectory: true);
      ensureScopeCurrent();
      return channels;
    },
    fetchChannel: (channelId) async {
      ensureScopeCurrent();
      final channel = await ref
          .read(channelsProvider.notifier)
          .fetchOpenChannel(channelId);
      ensureScopeCurrent();
      return channel;
    },
    createChannel:
        ({
          required channelId,
          required name,
          required channelType,
          required visibility,
          description,
          ttlSeconds,
        }) => channelActions.createChannel(
          channelId: channelId,
          name: name,
          channelType: channelType,
          visibility: visibility,
          description: description,
          ttlSeconds: ttlSeconds,
        ),
    joinChannel: channelActions.joinChannel,
    relayHttpOrigin: scope.relayHttpOrigin,
    isScopeCurrent: isScopeCurrent,
  );
}

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
    // The app rebuilds on every lifecycle change. A new ThemeData never
    // equals the last one, so MaterialApp would animate between identical
    // themes, and each frame of that makes every message re-parse its
    // markdown. Build the themes only when the accent changes.
    final accent = communityTheme.accent;
    final lightTheme = useMemoized(
      () => mobileAppearanceTheme(Brightness.light, accent),
      [accent],
    );
    final darkTheme = useMemoized(
      () => mobileAppearanceTheme(Brightness.dark, accent),
      [accent],
    );
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
      theme: lightTheme,
      darkTheme: darkTheme,
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
        // An internal build never offers pairing, even when auth state is
        // unreadable: pairing could make another relay the active community.
        error: (_, _) => OrganizationEnvironment.enabled
            ? const OrganizationAppEntry()
            : const PairingPage(),
        data: (state) => switch (state.status) {
          AuthStatus.authenticated => HostedAppGate(
            authenticated: true,
            child: CoveredRouteKeyboardInsets(
              child: HomePage(
                onAgents: showAgents,
                defaultAgentBuilder: (context) => DefaultAgentLauncher(
                  onChooseAgent: () => showAgents(context, createOnOpen: true),
                ),
                openBestieConversation: openBestieConversation,
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
