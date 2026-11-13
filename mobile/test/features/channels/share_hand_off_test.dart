import 'dart:convert';
import 'dart:io';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/features/channels/open_channel_routes.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/share/pending_share.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_storage_test.dart';

const _shareId = '0F8B5C2E-1D4A-4B7E-9C3F-2A6D8E1B4C70';

void main() {
  late Directory shares;

  setUp(() {
    shares = Directory.systemTemp.createTempSync('pending-shares-');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('buzz/app_group'), (
          call,
        ) async {
          expect(call.method, 'pendingSharesDirectory');
          return shares.path;
        });
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('buzz/app_group'), null);
    shares.deleteSync(recursive: true);
  });

  void writeShare(
    Map<String, Object?> manifest, {
    List<String> files = const [],
  }) {
    final directory = Directory('${shares.path}/$_shareId')..createSync();
    for (final name in files) {
      File('${directory.path}/$name').writeAsBytesSync([1, 2, 3]);
    }
    File(
      '${directory.path}/share.json',
    ).writeAsStringSync(jsonEncodeManifest(manifest));
  }

  Future<ProviderContainer> pumpDispatcher(
    WidgetTester tester, {
    void Function(NavigatorState navigator)? openPages,
  }) async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    await tester.runAsync(() async {
      await storage.save(_community);
      await storage.saveActiveId(_community.id);
    });
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        pendingDeepLinkProvider.overrideWith(
          () => _FakePendingDeepLinkNotifier(const ShareDeepLink(_shareId)),
        ),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier(Future.value([_channel])),
        ),
        relaySessionProvider.overrideWith(_ConnectedSession.new),
      ],
    );
    addTearDown(container.dispose);
    final navigatorKey = GlobalKey<NavigatorState>();
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          navigatorKey: navigatorKey,
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) => Text('opened ${channel.id}'),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );
    if (openPages != null) {
      openPages(navigatorKey.currentState!);
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
    }
    // The dispatcher reads the manifest with real file IO, so let it run
    // outside fake time until it has consumed the link.
    for (var i = 0; i < 100; i++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 20)),
      );
      await tester.pumpAndSettle();
      if (container.read(pendingDeepLinkProvider) == null) break;
    }
    return container;
  }

  testWidgets('a share to a suggested channel opens it with the items', (
    tester,
  ) async {
    writeShare(
      {
        'version': 1,
        'target': shareConversationIdentifier(
          communityId: _community.id,
          channelId: _channel.id,
        ),
        'text': ['https://example.org/a', 'Look'],
        'files': ['shot.png'],
      },
      files: ['shot.png'],
    );

    final container = await pumpDispatcher(tester);

    expect(find.text('opened channel-1'), findsOneWidget);
    expect(container.read(pendingDeepLinkProvider), isNull);
    final share = container
        .read(composerShareInboxProvider)['channel-1']!
        .single;
    expect(share.composerText, 'https://example.org/a\nLook');
    expect(share.files, ['${shares.path}/$_shareId/shot.png']);
  });

  testWidgets('a share to a conversation already open stays with its page', (
    tester,
  ) async {
    writeShare({
      'version': 1,
      'target': shareConversationIdentifier(
        communityId: _community.id,
        channelId: _channel.id,
      ),
      'text': ['Look'],
      'files': <String>[],
    });

    final container = await pumpDispatcher(
      tester,
      openPages: (navigator) => navigator
        ..push(MaterialPageRoute<void>(builder: (_) => const _OpenChannel()))
        ..push(
          MaterialPageRoute<void>(
            builder: (_) => const Scaffold(body: Text('another page')),
          ),
        ),
    );

    // One composer keeps owning the draft: nothing covering it is closed and
    // no second page for the channel opens.
    expect(find.text('another page'), findsOneWidget);
    expect(find.text('opened channel-1'), findsNothing);
    expect(
      find.text('Go back to that conversation to finish sharing'),
      findsOneWidget,
    );
    expect(
      container
          .read(composerShareInboxProvider)['channel-1']!
          .single
          .composerText,
      'Look',
    );
  });

  for (final (name, manifest) in [
    (
      'a conversation in a community this device no longer has',
      {
        'version': 1,
        'target': shareConversationIdentifier(
          communityId: 'signed-out-community',
          channelId: 'channel-1',
        ),
        'text': ['hello'],
        'files': <String>[],
      },
    ),
    (
      'a channel the community no longer lists',
      {
        'version': 1,
        'target': shareConversationIdentifier(
          communityId: 'community-1',
          channelId: 'deleted-channel',
        ),
        'text': ['hello'],
        'files': <String>[],
      },
    ),
    (
      'a manifest with a malformed missed count',
      {
        'version': 1,
        'text': ['hello'],
        'files': <String>[],
        'missed': 'one',
      },
    ),
    ('a manifest from an unknown version', {'version': 2}),
    ('a missing manifest', null),
  ]) {
    testWidgets('a share to $name is reported and dropped', (tester) async {
      if (manifest != null) writeShare(manifest);

      final container = await pumpDispatcher(tester);

      expect(find.text('That conversation isn’t available'), findsOneWidget);
      expect(container.read(pendingDeepLinkProvider), isNull);
      expect(container.read(composerShareInboxProvider), isEmpty);
      final directory = Directory('${shares.path}/$_shareId');
      for (var i = 0; i < 100 && directory.existsSync(); i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
      expect(directory.existsSync(), isFalse);
    });
  }

  test('share links accept only the extension’s share ids', () {
    for (final (uri, expected) in [
      ('com.example.buzz://share?id=$_shareId', const ShareDeepLink(_shareId)),
      ('buzz://share?id=$_shareId', const ShareDeepLink(_shareId)),
      ('com.example.buzz://share?id=../$_shareId', null),
      ('com.example.buzz://share?id=${_shareId.toLowerCase()}', null),
      ('com.example.buzz://share', null),
      ('com.example.buzz://share/extra?id=$_shareId', null),
      ('https://example.org/share?id=$_shareId', null),
    ]) {
      expect(parseBuzzDeepLink(Uri.parse(uri)), expected, reason: uri);
    }
  });
}

String jsonEncodeManifest(Map<String, Object?> manifest) =>
    const JsonEncoder().convert(manifest);

class _OpenChannel extends HookConsumerWidget {
  const _OpenChannel();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useOpenChannelRoute(ref, 'channel-1');
    return const Text('channel-1 already open');
  }
}

class _FakePendingDeepLinkNotifier extends PendingDeepLinkNotifier {
  _FakePendingDeepLinkNotifier(this.link);

  final BuzzDeepLink link;

  @override
  BuzzDeepLink? build() => link;
}

class _ConnectedSession extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  _FakeChannelsNotifier(this.channels);

  final Future<List<Channel>> channels;

  @override
  Future<List<Channel>> build() => channels;

  @override
  Future<Channel?> fetchOpenChannel(String channelId) async => null;
}

final _channel = Channel(
  id: 'channel-1',
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

final _community = Community(
  id: 'community-1',
  name: 'First',
  relayUrl: 'wss://first.example',
  addedAt: DateTime(2026),
);
