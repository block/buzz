import 'dart:async';

import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import 'features/activity/activity_provider.dart';
import 'features/activity/inbox_local_state_provider.dart';
import 'features/activity/inbox_read_state.dart';
import 'features/channels/channel.dart';
import 'features/channels/channel_management_provider.dart';
import 'features/channels/channels_provider.dart';
import 'features/channels/unread_badge/unread_badge_provider.dart';
import 'features/home/home_page.dart';
import 'features/invites/invite_create_page.dart';
import 'features/invites/invite_join_provider.dart';
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

const _starterChannelNamespace = '3ce33bea-8f09-5f1b-9c85-8a7d2659e6b0';

const _starterChannels = [
  (slug: 'general', description: 'General conversation and community updates.'),
  (
    slug: 'welcome-everyone',
    description: 'Say hi, ask a question, or share what brought you here.',
  ),
];

final _inviteRelayConnectedProvider = FutureProvider.family<void, String>((
  ref,
  expectedRelayUrl,
) async {
  final currentConfig = ref.read(relayConfigProvider);
  if (currentConfig.baseUrl != expectedRelayUrl) {
    throw StateError('Active community changed before invite recovery');
  }
  if (ref.read(relaySessionProvider).status == SessionStatus.connected) return;

  final connected = Completer<void>();
  ref.listen(relaySessionProvider, (_, next) {
    if (connected.isCompleted) return;
    if (ref.read(relayConfigProvider).baseUrl != expectedRelayUrl) {
      connected.completeError(
        StateError('Active community changed during invite recovery'),
      );
    } else if (next.status == SessionStatus.connected) {
      connected.complete();
    }
  });
  await connected.future;
});

/// App-level bridge from invite joining to the channels feature.
class MobileInviteJoinRecovery implements InviteJoinRecovery {
  final Future<List<Channel>> Function() _loadChannels;
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

  /// Creates recovery from channel-loading, creation, and join operations.
  const MobileInviteJoinRecovery({
    required Future<List<Channel>> Function() loadChannels,
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
  }) : _loadChannels = loadChannels,
       _createChannel = createChannel,
       _joinChannel = joinChannel,
       _relayHttpOrigin = relayHttpOrigin;

  /// Ensures memberships in the same public starter channels as desktop.
  ///
  /// Each channel is best-effort: one unavailable starter must not prevent the
  /// other from being joined or turn a successful invite claim into a failure.
  @override
  Future<String?> ensureStarterChannels() async {
    var channels = await _loadChannels();
    String? welcomeEveryoneId;

    for (final starter in _starterChannels) {
      try {
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
          } catch (error) {
            if (!_isDuplicateChannelError(error)) rethrow;
            channels = await _loadChannels();
            channel =
                _findStarterChannel(channels, starter.slug) ??
                channels
                    .where((candidate) => candidate.id == channelId)
                    .firstOrNull;
            if (channel == null) rethrow;
          }
        }

        if (!channel.isMember) await _joinChannel(channel.id);
        if (starter.slug == 'welcome-everyone') {
          welcomeEveryoneId = channel.id;
        }
      } catch (error) {
        debugPrint(
          '[InviteJoinRecovery] could not ensure #${starter.slug}: $error',
        );
      }
    }

    return welcomeEveryoneId;
  }
}

Channel? _findStarterChannel(List<Channel> channels, String name) {
  final normalizedName = name.trim().toLowerCase();
  return channels
      .where(
        (channel) =>
            channel.name.trim().toLowerCase() == normalizedName &&
            channel.isStream &&
            channel.visibility == 'open' &&
            !channel.isArchived,
      )
      .firstOrNull;
}

bool _isDuplicateChannelError(Object error) =>
    error.toString().contains('duplicate: channel already exists');

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

/// Builds invite recovery against the active app-level provider container.
InviteJoinRecovery buildMobileInviteJoinRecovery(Ref ref) {
  return MobileInviteJoinRecovery(
    loadChannels: () async {
      await ref.read(activeCommunityProvider.future);
      final relayUrl = ref.read(relayConfigProvider).baseUrl;
      await ref
          .read(_inviteRelayConnectedProvider(relayUrl).future)
          .timeout(const Duration(seconds: 15));
      await ref.read(channelsProvider.notifier).refresh();
      return ref.read(channelsProvider.future);
    },
    createChannel:
        ({
          required channelId,
          required name,
          required channelType,
          required visibility,
          description,
          ttlSeconds,
        }) => ref
            .read(channelActionsProvider)
            .createChannel(
              channelId: channelId,
              name: name,
              channelType: channelType,
              visibility: visibility,
              description: description,
              ttlSeconds: ttlSeconds,
            ),
    joinChannel: (channelId) =>
        ref.read(channelActionsProvider).joinChannel(channelId),
    relayHttpOrigin: ref.read(relayConfigProvider).baseUrl,
  );
}

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

Widget _buildSettingsPage(BuildContext context) => SettingsPage(
  profileHeader: const SettingsProfileHeader(),
  invitePageBuilder: (_) => const CommunityInvitePage(),
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
