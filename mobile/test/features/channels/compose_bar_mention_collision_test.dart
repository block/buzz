import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/photo_library.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/utils/string_utils.dart';

// Two agents wearing the same display name — the shape a duplicated agent
// identity takes in a channel roster. Without the npub line these two rows
// are pixel-identical.
const _liveAgentPubkey =
    '85f63083f4702f94bdb20e604815b03cd7b30ee349333de1a271d7d52731bae5';
const _twinAgentPubkey =
    '5228041f95e16eb8cf0f1b8ade09c8f1499a062e001742e38ca3d54adc9ab5e3';
const _soloAgentPubkey =
    '09aa16bf25a0efd5aa894812a0ed21e8b491e47cf17ee9dca8df8007025848bb';

late SharedPreferences _testPrefs;

ChannelMember _bot(String pubkey, String displayName) => ChannelMember(
  pubkey: pubkey,
  role: 'bot',
  joinedAt: DateTime.utc(2026),
  displayName: displayName,
);

Widget _buildComposeBar({required List<ChannelMember> members}) {
  return ProviderScope(
    overrides: [
      customEmojiListProvider.overrideWithValue(const <CustomEmoji>[]),
      photoLibraryProvider.overrideWithValue(const _EmptyPhotoLibrary()),
      currentPubkeyProvider.overrideWith((ref) => null),
      channelMembersProvider(
        'channel-1',
      ).overrideWith((ref) => Future.value(members)),
      agentDirectoryProvider.overrideWith(
        (ref) async => const <AgentDirectoryEntry>[],
      ),
      agentOwnersProvider.overrideWith((ref) async => const <String, String>{}),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
      relayConfigProvider.overrideWith(_FakeRelayConfigNotifier.new),
      savedPrefsProvider.overrideWithValue(_testPrefs),
      channelsProvider.overrideWith(() => _FakeChannelsNotifier(const [])),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(
        body: SafeArea(
          child: Align(
            alignment: Alignment.bottomCenter,
            child: ComposeBar(
              channelId: 'channel-1',
              onSend: (_, _, {mediaTags = const []}) async {},
            ),
          ),
        ),
      ),
    ),
  );
}

Future<void> _openMentions(WidgetTester tester, String query) async {
  await tester.tap(find.text('Message…'));
  await tester.pumpAndSettle();
  await tester.enterText(find.byType(TextField), query);
  await tester.pumpAndSettle();
}

void main() {
  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    _testPrefs = await SharedPreferences.getInstance();
  });

  testWidgets('same-named mention suggestions each show their npub', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildComposeBar(
        members: [
          _bot(_liveAgentPubkey, 'Fizz'),
          _bot(_twinAgentPubkey, 'Fizz'),
        ],
      ),
    );

    await _openMentions(tester, '@fiz');

    expect(find.text('Fizz'), findsNWidgets(2));
    expect(
      find.byKey(const ValueKey('mention-collision-npub')),
      findsNWidgets(2),
    );

    // Each row carries its own key, so the two are actually distinguishable
    // rather than merely annotated.
    final live = truncatePubkey(safeNpub(_liveAgentPubkey)!);
    final twin = truncatePubkey(safeNpub(_twinAgentPubkey)!);
    expect(live, isNot(twin));
    expect(find.text(live), findsOneWidget);
    expect(find.text(twin), findsOneWidget);
  });

  testWidgets('a uniquely named suggestion shows no npub', (tester) async {
    await tester.pumpWidget(
      _buildComposeBar(members: [_bot(_soloAgentPubkey, 'Honey')]),
    );

    await _openMentions(tester, '@hon');

    expect(find.text('Honey'), findsOneWidget);
    expect(find.byKey(const ValueKey('mention-collision-npub')), findsNothing);
  });
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => RelayConfig(
    baseUrl: 'http://localhost:3000',
    nsec: nostr.Keys.generate().nsec,
  );
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  final List<Channel> channels;

  _FakeChannelsNotifier(this.channels);

  @override
  Future<List<Channel>> build() async => channels;
}

class _EmptyPhotoLibrary implements PhotoLibrary {
  const _EmptyPhotoLibrary();

  @override
  Future<List<RecentPhoto>> loadRecentPhotos() async => const [];

  @override
  Future<List<XFile>> resolveSelectedPhotos(List<RecentPhoto> photos) async =>
      const [];
}
