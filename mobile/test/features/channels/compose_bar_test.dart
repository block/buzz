import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:image_picker/image_picker.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';

final _pngBytes = Uint8List.fromList([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x0d,
  0x49,
  0x48,
  0x44,
  0x52,
]);

final _gifBytes = Uint8List.fromList([
  0x47,
  0x49,
  0x46,
  0x38,
  0x39,
  0x61,
  0x01,
  0x00,
  0x01,
  0x00,
]);

final _apngBytes = Uint8List.fromList([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x08,
  0x61,
  0x63,
  0x54,
  0x4c,
  0x00,
  0x00,
  0x00,
  0x02,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x49,
  0x45,
  0x4e,
  0x44,
  0x00,
  0x00,
  0x00,
  0x00,
]);

const _mediaUploadPlatformChannel = MethodChannel('buzz/media_upload');

void _setMockMediaUploadPlatformHandler(
  Future<Object?> Function(MethodCall call)? handler,
) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_mediaUploadPlatformChannel, handler);
}

Widget _buildComposeBar({
  required MediaUploadService uploadService,
  required ComposeBarOnSend onSend,
}) {
  return ProviderScope(
    overrides: [
      mediaUploadServiceProvider.overrideWithValue(uploadService),
      currentPubkeyProvider.overrideWith((ref) => null),
      channelMembersProvider(
        'channel-1',
      ).overrideWith((ref) async => const <ChannelMember>[]),
      agentDirectoryProvider.overrideWith(
        (ref) async => const <AgentDirectoryEntry>[],
      ),
      agentOwnersProvider.overrideWith((ref) async => const <String, String>{}),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(
        body: SafeArea(
          child: ComposeBar(channelId: 'channel-1', onSend: onSend),
        ),
      ),
    ),
  );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() {
    _setMockMediaUploadPlatformHandler((call) async {
      switch (call.method) {
        case 'sanitizeImageForUpload':
          final arguments = call.arguments as Map<Object?, Object?>;
          return arguments['bytes'] as Uint8List;
        case 'transcodeImageToJpeg':
          return _pngBytes;
        default:
          return null;
      }
    });
  });

  tearDownAll(() {
    _setMockMediaUploadPlatformHandler(null);
  });

  group('ComposeBar', () {
    testWidgets('uploads an image and sends markdown plus imeta tags', (
      tester,
    ) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient((request) async {
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/test.png',
              'sha256':
                  '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
              'thumb': 'https://relay.example/media/test.thumb.jpg',
            }),
            200,
          );
        }),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_pngBytes, name: 'tiny.png'),
      );

      String? sentContent;
      List<List<String>> sentMediaTags = const [];
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {
                sentContent = content;
                sentMediaTags = mediaTags;
              },
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.paperclip));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Photo'));
      await tester.pumpAndSettle();

      expect(find.byTooltip('Remove attachment'), findsOneWidget);

      await tester.tap(find.byIcon(LucideIcons.sendHorizontal));
      await tester.pump();
      await tester.pumpAndSettle();

      expect(sentContent, '\n![image](https://relay.example/media/test.png)');
      expect(sentMediaTags, hasLength(1));
      expect(sentMediaTags.first.first, 'imeta');
      expect(
        sentMediaTags.first,
        contains('url https://relay.example/media/test.png'),
      );
      expect(find.byTooltip('Remove attachment'), findsNothing);
    });

    testWidgets('keeps the remove button pinned to the attachment corner', (
      tester,
    ) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient((request) async {
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/test.png',
              'sha256':
                  '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          );
        }),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_pngBytes, name: 'tiny.png'),
      );

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.paperclip));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Photo'));
      await tester.pumpAndSettle();

      final attachmentFinder = find.byKey(
        const ValueKey(
          'compose-attachment:https://relay.example/media/test.png',
        ),
      );
      final removeButtonFinder = find.byTooltip('Remove attachment');

      expect(attachmentFinder, findsOneWidget);
      expect(removeButtonFinder, findsOneWidget);

      final attachmentTopRight = tester.getTopRight(attachmentFinder);
      final attachmentTopLeft = tester.getTopLeft(attachmentFinder);
      final removeButtonCenter = tester.getCenter(removeButtonFinder);

      expect(
        attachmentTopRight.dx - removeButtonCenter.dx,
        lessThanOrEqualTo(16),
      );
      expect(
        removeButtonCenter.dy - attachmentTopLeft.dy,
        lessThanOrEqualTo(16),
      );
    });

    testWidgets('shows an upload error when gallery upload fails', (
      tester,
    ) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient((request) async {
          return http.Response('bad upload', 401);
        }),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_pngBytes, name: 'tiny.png'),
      );

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.paperclip));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Photo'));
      await tester.pumpAndSettle();

      expect(find.textContaining('upload failed'), findsOneWidget);
    });

    testWidgets('shows a clean error when a GIF is picked', (tester) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_gifBytes, name: 'animated.gif'),
      );

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.paperclip));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Photo'));
      await tester.pumpAndSettle();

      expect(
        find.textContaining('GIF uploads are not supported on mobile yet'),
        findsOneWidget,
      );
    });

    testWidgets('shows a clean error when an animated PNG is picked', (
      tester,
    ) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_apngBytes, name: 'animated.png'),
      );

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await tester.tap(find.byIcon(LucideIcons.paperclip));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Photo'));
      await tester.pumpAndSettle();

      expect(
        find.textContaining('Animated PNG uploads are not supported on mobile'),
        findsOneWidget,
      );
    });

    // Skip: video upload relies on native platform bridging
    // (transcodeVideoToMp4) that can't be fully mocked in widget tests.
    testWidgets('taps Video in chooser sheet and uploads video', skip: true, (
      tester,
    ) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      // Build a temp file with a valid MP4 ftyp header (isom brand).
      final mp4Bytes = Uint8List(32);
      mp4Bytes[3] = 32;
      mp4Bytes[4] = 0x66; // f
      mp4Bytes[5] = 0x74; // t
      mp4Bytes[6] = 0x79; // y
      mp4Bytes[7] = 0x70; // p
      mp4Bytes[8] = 0x69; // i
      mp4Bytes[9] = 0x73; // s
      mp4Bytes[10] = 0x6F; // o
      mp4Bytes[11] = 0x6D; // m
      final tempDir = await Directory.systemTemp.createTemp('compose_video_');
      final tempFile = File('${tempDir.path}/clip.mp4');
      await tempFile.writeAsBytes(mp4Bytes);

      try {
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nsec,
          httpClient: http_testing.MockClient((request) async {
            return http.Response(
              jsonEncode({
                'url': 'https://relay.example/media/test.mp4',
                'sha256':
                    '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                'size': 1024,
                'type': 'video/mp4',
                'uploaded': 1,
              }),
              200,
            );
          }),
          pickGalleryVideo: () async => XFile(tempFile.path),
          pickGalleryImage: () async => null,
        );

        String? sentContent;
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: uploadService,
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {
                  sentContent = content;
                },
          ),
        );

        await tester.tap(find.byIcon(LucideIcons.paperclip));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Video'));
        // Pump enough frames for the async file read + upload to complete.
        // Can't use pumpAndSettle here — the upload spinner's animation
        // prevents settling while the async upload is in-flight.
        for (var i = 0; i < 10; i++) {
          await tester.pump(const Duration(milliseconds: 100));
        }

        // Video attachment should show a video icon (not a broken image).
        expect(find.byIcon(LucideIcons.video), findsOneWidget);

        await tester.tap(find.byIcon(LucideIcons.sendHorizontal));
        await tester.pump();
        await tester.pumpAndSettle();

        expect(sentContent, '\n![video](https://relay.example/media/test.mp4)');
      } finally {
        await tempDir.delete(recursive: true);
      }
    });
  });

  group('findTrigger', () {
    test('finds @ at start of text', () {
      expect(findTrigger('@alice', 6, '@', stopAtSpace: false), 0);
    });

    test('finds @ after a space', () {
      expect(findTrigger('hello @bob', 10, '@', stopAtSpace: false), 6);
    });

    test('finds @ after a newline', () {
      expect(findTrigger('line1\n@bob', 10, '@', stopAtSpace: false), 6);
    });

    test('returns null when @ is mid-word (no word boundary)', () {
      expect(findTrigger('foo@bar', 7, '@', stopAtSpace: false), isNull);
    });

    test('@ with stopAtSpace:false walks through spaces', () {
      // "@Alice Smith" — cursor at end, should find @ at index 0.
      expect(findTrigger('@Alice Smith', 12, '@', stopAtSpace: false), 0);
    });

    test('@ with stopAtSpace:false walks through spaces after prefix', () {
      expect(findTrigger('hey @Alice Smith', 16, '@', stopAtSpace: false), 4);
    });

    test('finds # at start of text', () {
      expect(findTrigger('#general', 8, '#'), 0);
    });

    test('finds # after a space', () {
      expect(findTrigger('hello #general', 14, '#'), 6);
    });

    test('# with stopAtSpace:true stops at space', () {
      // "hello #chan name" with cursor at end — space stops the walk before #.
      expect(findTrigger('hello #chan name', 15, '#'), isNull);
    });

    test('# stops at newline', () {
      expect(findTrigger('line1\n#foo', 10, '#'), 6);
    });

    test('returns null when # is mid-word', () {
      expect(findTrigger('foo#bar', 7, '#'), isNull);
    });

    test('returns null when cursor is 0', () {
      expect(findTrigger('@hello', 0, '@'), isNull);
    });

    test('returns null on empty text', () {
      expect(findTrigger('', 0, '@'), isNull);
    });

    test('finds trigger right at cursor boundary', () {
      // cursor=1, text="#", should find # at index 0.
      expect(findTrigger('#', 1, '#'), 0);
    });
  });

  group('filterChannels', () {
    final channels = [
      _makeChannel(name: 'general', channelType: 'stream'),
      _makeChannel(name: 'random', channelType: 'stream'),
      _makeChannel(name: 'announcements', channelType: 'forum'),
      _makeChannel(name: 'design-team', channelType: 'stream'),
      _makeChannel(name: 'dm-alice-bob', channelType: 'dm'),
    ];

    test('returns empty list when query is null', () {
      expect(filterChannels(channels, null), isEmpty);
    });

    test('returns all non-DM channels when query is empty string', () {
      final result = filterChannels(channels, '');
      expect(result.length, 4);
      expect(result.every((c) => c.channelType != 'dm'), isTrue);
    });

    test('filters by substring match', () {
      final result = filterChannels(channels, 'gen');
      expect(result.length, 1);
      expect(result.first.name, 'general');
    });

    test('is case-insensitive', () {
      final result = filterChannels(channels, 'RANDOM');
      expect(result.length, 1);
      expect(result.first.name, 'random');
    });

    test('excludes DM channels', () {
      final result = filterChannels(channels, 'dm');
      expect(result, isEmpty);
    });

    test('matches partial channel names', () {
      final result = filterChannels(channels, 'design');
      expect(result.length, 1);
      expect(result.first.name, 'design-team');
    });

    test('limits to 8 results', () {
      final manyChannels = List.generate(
        15,
        (i) => _makeChannel(name: 'channel-$i', channelType: 'stream'),
      );
      final result = filterChannels(manyChannels, '');
      expect(result.length, 8);
    });
  });

  group('spliceAndMoveCursor', () {
    test('replaces text range and moves cursor', () {
      // Simulates "@ali|" with cursor right after the query (no trailing space
      // in the query portion). The replacement includes a trailing space, so
      // the original space before "world" is preserved as-is.
      final controller = TextEditingController(text: 'hello @ali world');
      controller.selection = const TextSelection.collapsed(offset: 10);

      spliceAndMoveCursor(
        controller,
        FocusNode(),
        start: 6,
        replacement: '@Alice ',
      );

      // [start=6, cursor=10) → "hello " + "@Alice " + " world"
      expect(controller.text, 'hello @Alice  world');
      expect(controller.selection.baseOffset, 13); // after "@Alice "
    });

    test('replaces #channel query with channel name', () {
      final controller = TextEditingController(text: 'see #gen for details');
      controller.selection = const TextSelection.collapsed(offset: 8);

      spliceAndMoveCursor(
        controller,
        FocusNode(),
        start: 4,
        replacement: '#general ',
      );

      expect(controller.text, 'see #general  for details');
      expect(controller.selection.baseOffset, 13); // after "#general "
    });

    test('handles replacement at start of text', () {
      final controller = TextEditingController(text: '@bo rest');
      controller.selection = const TextSelection.collapsed(offset: 3);

      spliceAndMoveCursor(
        controller,
        FocusNode(),
        start: 0,
        replacement: '@Bob ',
      );

      expect(controller.text, '@Bob  rest');
      expect(controller.selection.baseOffset, 5);
    });

    test('handles replacement at end of text', () {
      final controller = TextEditingController(text: 'hello #gen');
      controller.selection = const TextSelection.collapsed(offset: 10);

      spliceAndMoveCursor(
        controller,
        FocusNode(),
        start: 6,
        replacement: '#general ',
      );

      expect(controller.text, 'hello #general ');
      expect(controller.selection.baseOffset, 15);
    });

    test('clamps start to text bounds', () {
      final controller = TextEditingController(text: 'hi');
      controller.selection = const TextSelection.collapsed(offset: 2);

      // start beyond text length should be clamped
      spliceAndMoveCursor(
        controller,
        FocusNode(),
        start: 0,
        replacement: '@Name ',
      );

      expect(controller.text, '@Name ');
      expect(controller.selection.baseOffset, 6);
    });
  });
}

Channel _makeChannel({required String name, required String channelType}) {
  return Channel(
    id: 'id-$name',
    name: name,
    channelType: channelType,
    visibility: 'open',
    description: '',
    createdBy: 'pubkey123',
    createdAt: DateTime(2024),
    memberCount: 5,
  );
}
