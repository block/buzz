import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/features/channels/me/me_unread_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'sidebar_sync_fixture.dart';

const _hidden =
    '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798';
const _starred =
    'c6047f9441ed7d6d3045406e95c07cd85a778e4b8cef3ca7abac09b95c709ee5';
const _avatarUrl = 'https://images.example/hidden-user.png';

void main() {
  testWidgets('saved-collapsed DMs never mount on launch', (tester) async {
    tester.view.physicalSize = const Size(1170, 2600);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    final relay = SidebarRelay();
    relay.session.debugRelayUrl = 'https://relay.example';
    SharedPreferences.setMockInitialValues({
      'collapsed-sections:https://relay.example:${relay.pubkey}': ['dms'],
    });
    final prefs = await SharedPreferences.getInstance();
    final avatarRequests = <Uri>[];
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        userCacheProvider.overrideWith(_ProfileCache.new),
        mediaHttpClientProvider.overrideWithValue(
          MockClient((request) async {
            if (request.url.toString() == _avatarUrl) {
              avatarRequests.add(request.url);
            }
            return http.Response('', 404);
          }),
        ),
        relayConfigProvider.overrideWith(() => _Config(relay.keys.nsec)),
        relaySessionProvider.overrideWith(() => relay.session),
        activeCommunityProvider.overrideWith(
          (ref) async => Community(
            id: 'example',
            name: 'Example',
            relayUrl: 'https://relay.example',
            addedAt: DateTime(2026),
          ),
        ),
        channelsProvider.overrideWith(_Channels.new),
        meGroupsProvider.overrideWith(
          (ref) => Stream.value(const MeGroups(groups: [])),
        ),
        defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
        meAssistantsProvider.overrideWith((ref) async => []),
        channelStarsProvider.overrideWith(_Stars.new),
        meUnreadChannelIdsProvider.overrideWith(_SoloUnread.new),
      ],
    );
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          theme: AppTheme.light(),
          home: HomePage(
            settingsPageBuilder: (_) => const SizedBox(),
            onAgents: (_) {},
            defaultAgentBuilder: (_) => const SizedBox(),
            openBestieConversation: (_, _, _) async {},
            mePageBuilder: (_, chat, project) => MePage(
              sliver: true,
              newChatAction: chat,
              newWorkspaceAction: project,
            ),
          ),
        ),
      ),
    );

    final hiddenAvatar = find.byWidgetPredicate(
      (widget) => widget is MediaImage && widget.url == _avatarUrl,
    );
    List<Set<String>> presenceRequests() => [
      for (final request in relay.reqs)
        for (final filter in request.skip(2).whereType<Map>())
          if ((filter['kinds'] as List?)?.contains(EventKind.presenceUpdate) ==
              true)
            (filter['authors'] as List).cast<String>().toSet(),
    ];
    for (var i = 0; i < 35; i++) {
      await tester.pump(const Duration(milliseconds: 16));
      expect(hiddenAvatar, findsNothing);
    }
    expect(find.text('Direct messages'), findsOneWidget);
    expect(avatarRequests, isEmpty);
    expect(presenceRequests(), [
      {_starred},
    ]);

    // A hidden row that mounted would retire its presence key here.
    await tester.pump(const Duration(seconds: 31));
    await tester.pump();
    expect(presenceRequests(), [
      {_starred},
    ]);
    await tester.pumpWidget(const SizedBox.shrink());
    container.dispose();
  });
}

class _ProfileCache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {
    _starred: const UserProfile(pubkey: _starred, displayName: 'Starred user'),
    _hidden: const UserProfile(
      pubkey: _hidden,
      displayName: 'Hidden user',
      avatarUrl: _avatarUrl,
    ),
  };
}

Channel _dm(String id, String participant) => Channel(
  id: id,
  name: id,
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: participant,
  createdAt: DateTime(2026),
  memberCount: 2,
  participantPubkeys: [participant],
  isMember: true,
);

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: 'general',
      name: 'general',
      channelType: 'stream',
      visibility: 'private',
      description: '',
      createdBy: 'other',
      createdAt: DateTime(2026),
      memberCount: 2,
      isMember: true,
    ),
    _dm('hidden-dm', _hidden),
    _dm('pinned', _starred),
  ];
}

class _Stars extends ChannelStarsNotifier {
  @override
  ChannelStarsState build() => const ChannelStarsState(
    isReady: true,
    store: ChannelStarStore(
      channels: {'pinned': ChannelStarEntry(starred: true, updatedAt: 1)},
    ),
  );
}

class _Config extends RelayConfigNotifier {
  _Config(this._nsec);
  final String _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

class _SoloUnread extends MeUnreadChannelIds {
  @override
  Set<String> build() => const {};
}
