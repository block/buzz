import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/features/channels/share_target_sheet.dart';
import 'package:buzz/features/channels/share_targets_sync.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/composer/composer_seed_provider.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/share_inbox/share_inbox_provider.dart';
import 'package:buzz/shared/share_inbox/share_targets.dart';
import 'package:buzz/shared/share_inbox/shared_payload.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  testWidgets('routes a targeted share straight into its channel', (
    tester,
  ) async {
    final harness = await _Harness.create(
      initial: _payload('a', channelId: _general.id),
    );
    await tester.pumpWidget(harness.app());
    await tester.pumpAndSettle();

    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, _general.id);
    expect(destination.link, ChannelDeepLink(channelId: _general.id));
    expect(find.byType(ShareTargetSheet), findsNothing);
    expect(harness.container.read(composerSeedProvider)[_general.id], [
      const ComposerSeedFile(
        path: '/inbox/a/photo.jpg',
        name: 'photo.jpg',
        mimeType: 'image/jpeg',
      ),
    ]);
    expect(harness.discarded, ['a']);
    expect(harness.container.read(pendingSharedPayloadProvider), isNull);

    // While signed in, the extension's destination list is kept current.
    await tester.pump(ShareTargetsSyncNotifier.debounce);
    expect(harness.published.last.targets.map((t) => t.channelId), [
      _general.id,
    ]);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('falls back to the picker when the target is not joinable', (
    tester,
  ) async {
    final harness = await _Harness.create(
      initial: _payload('a', channelId: _notJoined.id),
    );
    await tester.pumpWidget(harness.app());
    await tester.pumpAndSettle();

    expect(find.byType(_CapturedDestination), findsNothing);
    expect(find.byType(ShareTargetSheet), findsOneWidget);
    expect(harness.container.read(composerSeedProvider), isEmpty);
    expect(harness.discarded, isEmpty);
  });

  testWidgets('dismissing the picker discards that payload', (tester) async {
    final harness = await _Harness.create(initial: _payload('a'));
    // A share that lands during onboarding waits until dispatch turns on.
    await tester.pumpWidget(harness.app(dispatch: false));
    await tester.pumpAndSettle();
    expect(find.byType(ShareTargetSheet), findsNothing);
    // Channels were already loaded behind onboarding; no load event follows.
    await harness.container.read(channelsProvider.future);
    await tester.pump();

    await tester.pumpWidget(harness.app());
    await tester.pumpAndSettle();
    expect(find.byType(ShareTargetSheet), findsOneWidget);

    Navigator.of(tester.element(find.byType(ShareTargetSheet))).pop();
    await tester.pumpAndSettle();

    expect(find.byType(ShareTargetSheet), findsNothing);
    expect(harness.discarded, ['a']);
    expect(harness.container.read(pendingSharedPayloadProvider), isNull);
  });

  testWidgets(
    'a share arriving while the picker is open is kept and presented next',
    (tester) async {
      final harness = await _Harness.create(initial: _payload('a'));
      await tester.pumpWidget(harness.app());
      await tester.pumpAndSettle();
      expect(find.byType(ShareTargetSheet), findsOneWidget);

      // A newer share lands (foreground re-check) while A's picker is open.
      harness.payloads.replace(_payload('b'));
      await tester.pump();

      await tester.tap(find.byKey(ValueKey('share-target-${_general.id}')));
      await tester.pumpAndSettle();

      // Only A was released; B survives and gets its own picker.
      expect(harness.discarded, ['a']);
      expect(harness.container.read(pendingSharedPayloadProvider)?.id, 'b');
      final sheet = tester.widget<ShareTargetSheet>(
        find.byType(ShareTargetSheet),
      );
      expect(sheet.payload.id, 'b');
    },
  );
}

SharedPayload _payload(String id, {String? channelId}) => SharedPayload(
  id: id,
  items: [
    SharedItem(
      kind: SharedItemKind.file,
      path: '/inbox/$id/photo.jpg',
      name: 'photo.jpg',
      mimeType: 'image/jpeg',
    ),
  ],
  communityId: channelId == null ? null : _community.id,
  channelId: channelId,
);

class _Harness {
  _Harness(this.container, this.payloads, this.discarded, this.published);

  final ProviderContainer container;
  final _FakePayloadNotifier payloads;
  final List<String> discarded;
  final List<ShareTargetsSnapshot> published;

  static Future<_Harness> create({required SharedPayload initial}) async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    await storage.save(_community);
    await storage.saveActiveId(_community.id);
    final discarded = <String>[];
    final published = <ShareTargetsSnapshot>[];
    final payloads = _FakePayloadNotifier(initial);
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        pendingSharedPayloadProvider.overrideWith(() => payloads),
        sharedPayloadDiscarderProvider.overrideWithValue((id) async {
          discarded.add(id);
        }),
        shareTargetsSyncerProvider.overrideWithValue((snapshot) async {
          published.add(snapshot);
        }),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier([_general, _notJoined]),
        ),
        myPubkeyProvider.overrideWithValue('me'),
      ],
    );
    addTearDown(container.dispose);
    return _Harness(container, payloads, discarded, published);
  }

  Widget app({bool dispatch = true}) => UncontrolledProviderScope(
    container: container,
    child: MaterialApp(
      theme: AppTheme.light(),
      home: DeepLinkDispatcher(
        dispatchMessageLinks: dispatch,
        destinationBuilder: (channel, link) =>
            _CapturedDestination(channel: channel, link: link),
        child: const Scaffold(body: SizedBox()),
      ),
    ),
  );
}

/// Production notifier with the native inbox replaced by a test-set state.
class _FakePayloadNotifier extends PendingSharedPayloadNotifier {
  _FakePayloadNotifier(this._initial);

  final SharedPayload _initial;

  @override
  SharedPayload? build() => _initial;

  @override
  Future<void> sync() async {}

  void replace(SharedPayload payload) => state = payload;
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  _FakeChannelsNotifier(this.channels);

  final List<Channel> channels;

  @override
  Future<List<Channel>> build() async => channels;
}

class _CapturedDestination extends StatelessWidget {
  const _CapturedDestination({required this.channel, required this.link});

  final Channel channel;
  final BuzzDeepLink link;

  @override
  Widget build(BuildContext context) => const SizedBox();
}

final _community = Community(
  id: 'community-1',
  name: 'Tecnocracia',
  relayUrl: 'wss://community.example',
  addedAt: DateTime(2026),
);

final _general = Channel(
  id: '580ca78b-9dae-46f3-8854-bd671853ba32',
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 3,
  isMember: true,
);

final _notJoined = Channel(
  id: '7d4c1b52-0a3e-4f6b-9c2d-1e8f5a6b7c90',
  name: 'announcements',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 3,
  isMember: false,
);
