import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
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
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/photo_library.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/anchored_popover_menu.dart';
import 'package:buzz/shared/widgets/mobile_tab_footer_backdrop.dart';
import 'package:shared_preferences/shared_preferences.dart';

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
  ...ascii.encode('GIF89a'),
  0x02,
  0x00,
  0x02,
  0x00,
  0x80,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0xff,
  0xff,
  0xff,
  0x21,
  0xfe,
  0x05,
  ...ascii.encode('hello'),
  0x00,
  0x21,
  0xff,
  0x0b,
  ...ascii.encode('NETSCAPE2.0'),
  0x03,
  0x01,
  0x00,
  0x00,
  0x00,
  0x21,
  0xf9,
  0x04,
  0x00,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x2c,
  0x00,
  0x00,
  0x00,
  0x00,
  0x02,
  0x00,
  0x02,
  0x00,
  0x00,
  0x02,
  0x02,
  0x44,
  0x01,
  0x00,
  0x3b,
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
  ..._testPngChunk('acTL', [0, 0, 0, 2, 0, 0, 0, 0]),
  ..._testPngChunk('IEND', const []),
]);

List<int> _testPngChunk(String type, List<int> payload) {
  return [
    payload.length >> 24 & 0xff,
    payload.length >> 16 & 0xff,
    payload.length >> 8 & 0xff,
    payload.length & 0xff,
    ...ascii.encode(type),
    ...payload,
    0,
    0,
    0,
    0,
  ];
}

const _mediaUploadPlatformChannel = MethodChannel('buzz/media_upload');
const _nativeAttachmentPopoverChannel = MethodChannel(
  'buzz/native_attachment_popover',
);

void _setMockMediaUploadPlatformHandler(
  Future<Object?> Function(MethodCall call)? handler,
) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_mediaUploadPlatformChannel, handler);
}

void _setMockNativeAttachmentPopoverHandler(
  Future<Object?> Function(MethodCall call)? handler,
) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_nativeAttachmentPopoverChannel, handler);
}

Future<void> _sendNativeAttachmentPopoverCall(
  WidgetTester tester,
  String method, [
  Object? arguments,
]) async {
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    _nativeAttachmentPopoverChannel.name,
    _nativeAttachmentPopoverChannel.codec.encodeMethodCall(
      MethodCall(method, arguments),
    ),
    null,
  );
}

/// Shared mock prefs for the compose bar's draft store. Initialized in
/// [main].
late SharedPreferences _testPrefs;

Widget _buildComposeBar({
  required MediaUploadService uploadService,
  required ComposeBarOnSend onSend,
  List<ChannelMember> members = const <ChannelMember>[],
  Future<List<ChannelMember>>? membersFuture,
  List<AgentDirectoryEntry> relayAgents = const <AgentDirectoryEntry>[],
  List<Channel> channels = const <Channel>[],
  String? currentPubkey,
  bool? supportsShowingSystemContextMenu,
  TextScaler? textScaler,
  List<CustomEmoji> customEmoji = const <CustomEmoji>[],
  RelayConfigNotifier Function()? relayConfig,
  PhotoLibrary photoLibrary = const _EmptyPhotoLibrary(),
}) {
  return ProviderScope(
    overrides: [
      customEmojiListProvider.overrideWithValue(customEmoji),
      mediaUploadServiceProvider.overrideWithValue(uploadService),
      photoLibraryProvider.overrideWithValue(photoLibrary),
      currentPubkeyProvider.overrideWith((ref) => currentPubkey),
      channelMembersProvider(
        'channel-1',
      ).overrideWith((ref) => membersFuture ?? Future.value(members)),
      agentDirectoryProvider.overrideWith((ref) async => relayAgents),
      agentOwnersProvider.overrideWith((ref) async => const <String, String>{}),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
      relayConfigProvider.overrideWith(
        relayConfig ?? _FakeRelayConfigNotifier.new,
      ),
      savedPrefsProvider.overrideWithValue(_testPrefs),
      channelsProvider.overrideWith(() => _FakeChannelsNotifier(channels)),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      builder: supportsShowingSystemContextMenu == null && textScaler == null
          ? null
          : (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(
                supportsShowingSystemContextMenu:
                    supportsShowingSystemContextMenu ??
                    MediaQuery.of(context).supportsShowingSystemContextMenu,
                textScaler: textScaler ?? MediaQuery.textScalerOf(context),
              ),
              child: child!,
            ),
      home: Scaffold(
        body: SafeArea(
          child: Align(
            alignment: Alignment.bottomCenter,
            child: ComposeBar(channelId: 'channel-1', onSend: onSend),
          ),
        ),
      ),
    ),
  );
}

Widget _buildNativePopoverOwnershipHarness({
  required MediaUploadService uploadService,
  required bool includeFirstComposer,
}) {
  return ProviderScope(
    overrides: [
      customEmojiListProvider.overrideWithValue(const []),
      mediaUploadServiceProvider.overrideWithValue(uploadService),
      photoLibraryProvider.overrideWithValue(const _EmptyPhotoLibrary()),
      currentPubkeyProvider.overrideWith((ref) => null),
      channelMembersProvider(
        'channel-1',
      ).overrideWith((ref) => Future.value(const <ChannelMember>[])),
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
        body: Column(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            if (includeFirstComposer)
              ComposeBar(
                key: const ValueKey('first-composer'),
                channelId: 'channel-1',
                onSend: (_, _, {mediaTags = const []}) async {},
              ),
            ComposeBar(
              key: const ValueKey('second-composer'),
              channelId: 'channel-1',
              onSend: (_, _, {mediaTags = const []}) async {},
            ),
          ],
        ),
      ),
    ),
  );
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => RelayConfig(
    baseUrl: 'http://localhost:3000',
    nsec: nostr.Keys.generate().nsec,
  );
}

class _EmptyPhotoLibrary implements PhotoLibrary {
  const _EmptyPhotoLibrary();

  @override
  Future<List<RecentPhoto>> loadRecentPhotos() async => const [];

  @override
  Future<List<XFile>> resolveSelectedPhotos(List<RecentPhoto> photos) async =>
      const [];
}

class _FakeVideoUploadService extends MediaUploadService {
  final XFile video;
  XFile? uploadedVideo;

  _FakeVideoUploadService(this.video)
    : super(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );

  @override
  Future<XFile?> pickGalleryVideo() async => video;

  @override
  Future<BlobDescriptor> uploadVideo(
    XFile pickedVideo, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    uploadedVideo = pickedVideo;
    onProgress?.call(1);
    return const BlobDescriptor(
      url: 'https://relay.example/media/test.mp4',
      sha256:
          '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
      size: 32,
      type: 'video/mp4',
      uploaded: 1,
    );
  }
}

class _FakePhotoLibrary implements PhotoLibrary {
  final List<RecentPhoto> photos;

  const _FakePhotoLibrary(this.photos);

  @override
  Future<List<RecentPhoto>> loadRecentPhotos() async => photos;

  @override
  Future<List<XFile>> resolveSelectedPhotos(List<RecentPhoto> photos) async => [
    for (final photo in photos)
      XFile.fromData(_pngBytes, name: '${photo.id}.png'),
  ];
}

/// Relay config that starts from a fixed identity and can be switched
/// in place via [RelayConfigNotifier.update] — simulates a community or
/// account switch while widgets stay mounted.
class _SwitchableRelayConfigNotifier extends RelayConfigNotifier {
  final RelayConfig initial;

  _SwitchableRelayConfigNotifier(this.initial);

  @override
  RelayConfig build() => initial;
}

class _RecordingRelaySocket extends RelaySocket {
  final List<Map<String, dynamic>> events;
  final void Function(List<dynamic> message) handleMessage;

  /// Invoked after an event has been recorded and acknowledged, before the
  /// caller's `await` resumes. Lets a test interleave state changes (such as
  /// a community switch) between two relay round trips.
  final void Function(Map<String, dynamic> event)? onEventAcknowledged;

  _RecordingRelaySocket(
    this.events,
    this.handleMessage, {
    this.onEventAcknowledged,
  }) : super(
         wsUrl: 'ws://localhost',
         nsec: null,
         onMessage: handleMessage,
         onConnected: () {},
         onDisconnected: (_) {},
       );

  @override
  SocketState get state => SocketState.connected;

  @override
  void send(List<dynamic> payload) {
    if (payload case ['EVENT', final Map<String, dynamic> event]) {
      events.add(event);
      final id = event['id'] as String;
      super.debugHandleOkForTest(['OK', id, true, '']);
      onEventAcknowledged?.call(event);
    }
  }

  @override
  Future<void> disconnect() async {}

  @override
  void dispose() {}
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  final List<Channel> _channels;

  _FakeChannelsNotifier(this._channels);

  @override
  Future<List<Channel>> build() async => _channels;

  @override
  Future<void> refresh() async {
    state = AsyncData(_channels);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    _testPrefs = await SharedPreferences.getInstance();
  });

  setUpAll(() {
    _setMockMediaUploadPlatformHandler((call) async {
      switch (call.method) {
        case 'sanitizeImageForUpload':
          final arguments = call.arguments as Map<Object?, Object?>;
          return arguments['bytes'] as Uint8List;
        case 'transcodeImageToJpeg':
          return _pngBytes;
        case 'clipboardHasImage':
          return true;
        default:
          return null;
      }
    });
  });

  tearDownAll(() {
    _setMockMediaUploadPlatformHandler(null);
  });

  group('ComposeBar', () {
    testWidgets('starts compact and grows to the full-width composer', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      expect(find.byType(TextField), findsNothing);
      expect(find.byTooltip('Add attachment').hitTestable(), findsOneWidget);
      expect(find.byIcon(LucideIcons.arrowUp).hitTestable(), findsOneWidget);
      expect(find.byKey(const ValueKey('composer-footer-gradient')), findsOne);
      final composerBackdrop = find.descendant(
        of: find.byKey(const ValueKey('composer-footer-gradient')),
        matching: find.byType(MobileTabFooterBackdrop),
      );
      expect(composerBackdrop, findsOneWidget);
      expect(
        tester.getSize(composerBackdrop).height,
        mobileTabFooterBackdropHeight(tester.element(composerBackdrop)),
      );
      final compactDecoration =
          tester
                  .widget<Container>(
                    find.byKey(const ValueKey('composer-surface')),
                  )
                  .decoration
              as BoxDecoration;
      expect(
        compactDecoration.borderRadius,
        BorderRadius.circular(Radii.dialog + Grid.quarter),
      );
      final compactWidth = tester
          .getSize(find.byKey(const ValueKey('composer-width-transition')))
          .width;

      await _expandComposer(tester);

      final expandedWidth = tester
          .getSize(find.byKey(const ValueKey('composer-width-transition')))
          .width;
      final expandedDecoration =
          tester
                  .widget<Container>(
                    find.byKey(const ValueKey('composer-surface')),
                  )
                  .decoration
              as BoxDecoration;
      expect(compactWidth, closeTo(expandedWidth * 0.85, 0.5));
      expect(
        expandedDecoration.borderRadius,
        BorderRadius.circular(Radii.dialog),
      );
      expect(find.byType(TextField), findsOneWidget);
      expect(find.byIcon(LucideIcons.atSign), findsOneWidget);
      expect(find.byIcon(LucideIcons.hash), findsOneWidget);
      expect(find.byIcon(LucideIcons.smilePlus), findsOneWidget);
      expect(find.byIcon(LucideIcons.aLargeSmall), findsOneWidget);
    });

    testWidgets('returns to the compact capsule when the keyboard drops', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );
      await _expandComposer(tester);
      final focusNode = tester
          .widget<TextField>(find.byType(TextField))
          .focusNode!;
      expect(focusNode.hasFocus, isTrue);
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      addTearDown(tester.view.reset);
      await tester.pump();

      tester.view.viewInsets = FakeViewPadding.zero;
      await tester.pumpAndSettle();

      expect(find.byType(TextField), findsNothing);
      expect(focusNode.hasFocus, isFalse);
      final compactDecoration =
          tester
                  .widget<Container>(
                    find.byKey(const ValueKey('composer-surface')),
                  )
                  .decoration
              as BoxDecoration;
      expect(
        compactDecoration.borderRadius,
        BorderRadius.circular(Radii.dialog + Grid.quarter),
      );

      await tester.tap(find.text('Message\u2026'));
      await tester.pumpAndSettle();
      expect(find.byType(TextField), findsOneWidget);
      expect(
        tester.widget<TextField>(find.byType(TextField)).focusNode!.hasFocus,
        isTrue,
      );
    });

    testWidgets('attachment control responds while the composer is expanding', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      try {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(nostr.Keys.generate().nsec),
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await tester.tap(find.text('Message\u2026'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 80));
        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();

        expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('mounted composer does not carry draft text across an in-place '
        'identity switch', (tester) async {
      final keysA = nostr.Keys.generate();
      final keysB = nostr.Keys.generate();
      const relayUrl = 'http://localhost:3000';

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: MediaUploadService(
            baseUrl: relayUrl,
            nsec: keysA.nsec,
            pickGalleryImage: () async =>
                XFile.fromData(_pngBytes, name: 'identity-a.png'),
            pickGalleryVideo: () async => null,
          ),
          relayConfig: () => _SwitchableRelayConfigNotifier(
            RelayConfig(baseUrl: relayUrl, nsec: keysA.nsec),
          ),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      String? storedText(nostr.Keys keys) {
        final raw = _testPrefs.getString(
          'compose_drafts_v1:$relayUrl:${keys.public}',
        );
        if (raw == null) return null;
        final drafts = jsonDecode(raw) as List;
        if (drafts.isEmpty) return null;
        return (drafts.first as Map<String, dynamic>)['text'] as String?;
      }

      // Identity A types a draft; it persists into A's namespaced store.
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), 'identity A secret draft');
      await tester.pump();
      expect(storedText(keysA), 'identity A secret draft');
      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();
      expect(find.byTooltip('Remove attachment'), findsOneWidget);

      // Switch identity in place while the composer stays mounted.
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: relayUrl, nsec: keysB.nsec);
      await tester.pumpAndSettle();

      // The mounted composer must not carry identity A's text forward.
      await _expandComposer(tester);
      final textField = tester.widget<TextField>(find.byType(TextField));
      expect(textField.controller!.text, isEmpty);
      expect(storedText(keysB), isNull);
      expect(find.byTooltip('Remove attachment'), findsNothing);
      expect(
        find.byKey(const ValueKey('compose-upload-progress')),
        findsNothing,
      );

      // Identity B's edits persist only into B's store; A's is untouched.
      await tester.enterText(find.byType(TextField), 'identity B text');
      await tester.pump();
      expect(storedText(keysB), 'identity B text');
      expect(storedText(keysA), 'identity A secret draft');

      // Switching back restores identity A's own draft into the composer.
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: relayUrl, nsec: keysA.nsec);
      await tester.pumpAndSettle();
      expect(textField.controller!.text, 'identity A secret draft');
      expect(storedText(keysB), 'identity B text');
    });

    testWidgets('inserts a community emoji at the cursor from the action row', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          customEmoji: const [
            CustomEmoji(shortcode: 'meow', url: 'https://example.com/meow.png'),
          ],
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), 'hello world');
      final textField = tester.widget<TextField>(find.byType(TextField));
      textField.controller!.selection = const TextSelection.collapsed(
        offset: 6,
      );

      await tester.tap(find.byIcon(LucideIcons.smilePlus));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.sparkles));
      await tester.pump();
      await tester.tap(find.byTooltip(':meow:'));
      await tester.pumpAndSettle();

      expect(textField.controller!.text, 'hello :meow:world');
      expect(textField.controller!.selection.baseOffset, 12);
      expect(find.byType(TextField), findsOneWidget);
      expect(textField.focusNode!.hasFocus, isTrue);
    });

    testWidgets('composer controls use selection haptics', (tester) async {
      final hapticCalls = <MethodCall>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, (call) async {
            if (call.method == 'HapticFeedback.vibrate') {
              hapticCalls.add(call);
            }
            return null;
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(SystemChannels.platform, null),
      );

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );
      await _expandComposer(tester);
      hapticCalls.clear();

      await tester.tap(find.byIcon(LucideIcons.atSign));
      tester.widget<TextField>(find.byType(TextField)).controller!.clear();
      await tester.pump();
      await tester.tap(find.byIcon(LucideIcons.hash));
      await tester.pump();
      await tester.tap(find.byIcon(LucideIcons.aLargeSmall));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.bold));
      await tester.pump();
      await tester.tap(find.byTooltip('Close formatting'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Add attachment'));
      await tester.pumpAndSettle();

      expect(hapticCalls, hasLength(6));
      expect(
        hapticCalls.every(
          (call) => call.arguments == 'HapticFeedbackType.selectionClick',
        ),
        isTrue,
      );
    });

    testWidgets('composer suggestions use the shared popover treatment', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          channels: [_makeChannel(name: 'general', channelType: 'stream')],
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );
      await _expandComposer(tester);
      await tester.tap(find.byIcon(LucideIcons.hash));
      await tester.pumpAndSettle();

      final surface = find.byKey(const ValueKey('channel-suggestions-popover'));
      final material = tester.widget<Material>(surface);
      final shape = material.shape! as RoundedRectangleBorder;
      expect(shape.borderRadius, BorderRadius.circular(Radii.popover));
      expect(shape.side.color, Colors.black.withValues(alpha: 0.04));
      expect(material.elevation, appPopoverElevation);
      expect(
        material.shadowColor,
        appPopoverShadowColor(tester.element(surface)),
      );
      expect(
        tester.widget<Text>(find.text('general')).style?.fontFamily,
        'Inter',
      );
    });

    testWidgets('native All Photos picker failures show an error', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      _setMockNativeAttachmentPopoverHandler((call) async {
        return switch (call.method) {
          'isSupported' || 'present' => true,
          'dismiss' => null,
          _ => null,
        };
      });
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async =>
            throw PlatformException(code: 'photo_picker_failed'),
        pickGalleryVideo: () async => null,
      );

      try {
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

        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();
        await _sendNativeAttachmentPopoverCall(tester, 'pickAllPhotos');
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpAndSettle();

        expect(find.text('Unable to open your photo library.'), findsOneWidget);
      } finally {
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('opening the native attachment popover keeps composer focus', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      var presentCalls = 0;
      _setMockNativeAttachmentPopoverHandler((call) async {
        switch (call.method) {
          case 'isSupported':
            return true;
          case 'present':
            presentCalls += 1;
            return true;
          case 'dismiss':
            return null;
        }
        return null;
      });

      try {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(nostr.Keys.generate().nsec),
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await _expandComposer(tester);
        await tester.enterText(find.byType(TextField), 'Hello');
        await tester.pumpAndSettle();

        final textField = tester.widget<TextField>(find.byType(TextField));
        expect(textField.focusNode?.hasFocus, isTrue);

        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();

        expect(presentCalls, 1);
        expect(textField.focusNode?.hasFocus, isTrue);
      } finally {
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('leaving a focused composer dismisses the native keyboard', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      var dismissCalls = 0;
      _setMockNativeAttachmentPopoverHandler((call) async {
        switch (call.method) {
          case 'isSupported':
          case 'present':
            return true;
          case 'dismiss':
            dismissCalls += 1;
            return null;
        }
        return null;
      });

      try {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(nostr.Keys.generate().nsec),
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await _expandComposer(tester);
        final focusNode = tester
            .widget<TextField>(find.byType(TextField))
            .focusNode!;
        expect(focusNode.hasFocus, isTrue);

        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();
        expect(focusNode.hasFocus, isTrue);

        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();

        expect(focusNode.hasFocus, isFalse);
        expect(dismissCalls, 1);
      } finally {
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets(
      'unsupported iOS attachment popover unfocuses before fallback menu',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        _setMockNativeAttachmentPopoverHandler((call) async {
          return switch (call.method) {
            'isSupported' => false,
            'dismiss' => null,
            _ => null,
          };
        });

        try {
          await tester.pumpWidget(
            _buildComposeBar(
              uploadService: _testUploadService(nostr.Keys.generate().nsec),
              onSend:
                  (
                    content,
                    mentionPubkeys, {
                    mediaTags = const <List<String>>[],
                  }) async {},
            ),
          );

          await _expandComposer(tester);
          await tester.enterText(find.byType(TextField), 'Hello');
          await tester.pumpAndSettle();

          final textField = tester.widget<TextField>(find.byType(TextField));
          expect(textField.focusNode?.hasFocus, isTrue);

          await tester.tap(find.byTooltip('Add attachment').hitTestable());
          await tester.pumpAndSettle();

          expect(textField.focusNode?.hasFocus, isFalse);
          expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
        } finally {
          await tester.pumpWidget(const SizedBox.shrink());
          _setMockNativeAttachmentPopoverHandler(null);
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      },
    );

    testWidgets('disposing a non-owner keeps native popover callbacks active', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      var presentCalls = 0;
      var dismissCalls = 0;
      var pickAllPhotosCalls = 0;
      _setMockNativeAttachmentPopoverHandler((call) async {
        switch (call.method) {
          case 'isSupported':
            return true;
          case 'present':
            presentCalls += 1;
            return true;
          case 'dismiss':
            dismissCalls += 1;
            return null;
        }
        return null;
      });
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async {
          pickAllPhotosCalls += 1;
          return const [];
        },
        pickGalleryVideo: () async => null,
      );

      try {
        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: true,
          ),
        );

        await tester.tap(find.byTooltip('Add attachment').hitTestable().at(1));
        await tester.pumpAndSettle();
        expect(presentCalls, 1);

        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: false,
          ),
        );
        await tester.pumpAndSettle();
        expect(dismissCalls, 0);

        await _sendNativeAttachmentPopoverCall(tester, 'pickAllPhotos');
        await tester.pumpAndSettle();
        expect(pickAllPhotosCalls, 1);

        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
      } finally {
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets(
      'a pending native popover does not claim another composer tap',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        final supportResult = Completer<bool>();
        var supportCalls = 0;
        var presentCalls = 0;
        _setMockNativeAttachmentPopoverHandler((call) async {
          switch (call.method) {
            case 'isSupported':
              supportCalls += 1;
              return supportResult.future;
            case 'present':
              presentCalls += 1;
              return true;
            case 'dismiss':
              return null;
          }
          return null;
        });
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
          pickGalleryImage: () async => null,
          pickGalleryImages: () async => const [],
          pickGalleryVideo: () async => null,
        );

        try {
          await tester.pumpWidget(
            _buildNativePopoverOwnershipHarness(
              uploadService: uploadService,
              includeFirstComposer: true,
            ),
          );

          await tester.tap(
            find.byTooltip('Add attachment').hitTestable().at(0),
          );
          await tester.pump();
          await tester.tap(
            find.byTooltip('Add attachment').hitTestable().at(1),
          );
          await tester.pumpAndSettle();

          expect(supportCalls, 1);
          expect(presentCalls, 0);
          expect(
            find.descendant(
              of: find.byKey(const ValueKey('first-composer')),
              matching: find.byTooltip('Close attachments'),
            ),
            findsNothing,
          );
          expect(
            find.descendant(
              of: find.byKey(const ValueKey('second-composer')),
              matching: find.byTooltip('Close attachments'),
            ),
            findsWidgets,
          );

          supportResult.complete(true);
          await tester.pumpAndSettle();
          expect(presentCalls, 0);
          expect(
            find.descendant(
              of: find.byKey(const ValueKey('first-composer')),
              matching: find.byTooltip('Close attachments'),
            ),
            findsNothing,
          );
        } finally {
          if (!supportResult.isCompleted) supportResult.complete(false);
          await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
          await tester.pumpWidget(const SizedBox.shrink());
          _setMockNativeAttachmentPopoverHandler(null);
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      },
    );

    testWidgets('a repeated owner tap keeps its pending native presentation', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final supportResult = Completer<bool>();
      var supportCalls = 0;
      var presentCalls = 0;
      _setMockNativeAttachmentPopoverHandler((call) async {
        switch (call.method) {
          case 'isSupported':
            supportCalls += 1;
            return supportResult.future;
          case 'present':
            presentCalls += 1;
            return true;
          case 'dismiss':
            return null;
        }
        return null;
      });
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async => const [],
        pickGalleryVideo: () async => null,
      );

      try {
        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: false,
          ),
        );

        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pump();
        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();

        expect(supportCalls, 1);
        expect(presentCalls, 0);

        supportResult.complete(true);
        await tester.pumpAndSettle();

        expect(presentCalls, 1);
        expect(find.byTooltip('Close attachments'), findsNothing);
      } finally {
        if (!supportResult.isCompleted) supportResult.complete(false);
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('disposing the native popover owner releases ownership', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      var presentCalls = 0;
      var dismissCalls = 0;
      _setMockNativeAttachmentPopoverHandler((call) async {
        switch (call.method) {
          case 'isSupported':
            return true;
          case 'present':
            presentCalls += 1;
            return true;
          case 'dismiss':
            dismissCalls += 1;
            return null;
        }
        return null;
      });
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async => const [],
        pickGalleryVideo: () async => null,
      );

      try {
        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: true,
          ),
        );

        await tester.tap(find.byTooltip('Add attachment').hitTestable().at(0));
        await tester.pumpAndSettle();
        expect(presentCalls, 1);

        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: false,
          ),
        );
        await tester.pumpAndSettle();
        expect(dismissCalls, 1);

        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();
        expect(presentCalls, 2);
      } finally {
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('missing native dismiss bridge still releases ownership', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      var presentCalls = 0;
      _setMockNativeAttachmentPopoverHandler((call) async {
        switch (call.method) {
          case 'isSupported':
            return true;
          case 'present':
            presentCalls += 1;
            return true;
          case 'dismiss':
            throw MissingPluginException('dismiss is unavailable');
        }
        return null;
      });
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async => const [],
        pickGalleryVideo: () async => null,
      );

      try {
        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: true,
          ),
        );

        await tester.tap(find.byTooltip('Add attachment').hitTestable().at(0));
        await tester.pumpAndSettle();
        expect(presentCalls, 1);

        await tester.pumpWidget(
          _buildNativePopoverOwnershipHarness(
            uploadService: uploadService,
            includeFirstComposer: false,
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pumpAndSettle();
        expect(presentCalls, 2);
      } finally {
        await _sendNativeAttachmentPopoverCall(tester, 'dismissed');
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeAttachmentPopoverHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

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

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Remove attachment'), findsOneWidget);

      await _expandComposer(tester);
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
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

    testWidgets('uploads multiple system-selected photos in picker order', (
      tester,
    ) async {
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        httpClient: http_testing.MockClient((request) async {
          final mimeType = request.headers.entries
              .firstWhere((entry) => entry.key.toLowerCase() == 'content-type')
              .value;
          final isGif = mimeType == 'image/gif';
          return http.Response(
            jsonEncode({
              'url': isGif
                  ? 'https://relay.example/media/two.gif'
                  : 'https://relay.example/media/one.png',
              'sha256': isGif
                  ? '2222222222222222222222222222222222222222222222222222222222222222'
                  : '1111111111111111111111111111111111111111111111111111111111111111',
              'size': request.bodyBytes.length,
              'type': mimeType,
              'uploaded': 1,
            }),
            200,
          );
        }),
        pickGalleryImage: () async => null,
        pickGalleryImages: () async => [
          XFile.fromData(_pngBytes, name: 'one.png'),
          XFile.fromData(_gifBytes, name: 'two.gif'),
        ],
        pickGalleryVideo: () async => null,
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

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Remove attachment'), findsNWidgets(2));

      await _expandComposer(tester);
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();

      expect(
        sentContent,
        '\n![image](https://relay.example/media/one.png)'
        '\n![image](https://relay.example/media/two.gif)',
      );
      expect(sentMediaTags, hasLength(2));
      expect(sentMediaTags.map((tag) => tag[1]), [
        'url https://relay.example/media/one.png',
        'url https://relay.example/media/two.gif',
      ]);
    });

    testWidgets('defers system-selected photo uploads until send', (
      tester,
    ) async {
      final releaseFirstBatch = Completer<void>();
      var requestsStarted = 0;
      var activeRequests = 0;
      var peakActiveRequests = 0;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        httpClient: http_testing.MockClient((request) async {
          requestsStarted += 1;
          final requestNumber = requestsStarted;
          activeRequests += 1;
          peakActiveRequests = math.max(peakActiveRequests, activeRequests);
          if (requestNumber <= 3) {
            await releaseFirstBatch.future;
          }
          activeRequests -= 1;
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/photo-$requestNumber.png',
              'sha256':
                  '1111111111111111111111111111111111111111111111111111111111111111',
              'size': request.bodyBytes.length,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          );
        }),
        pickGalleryImage: () async => null,
        pickGalleryImages: () async => [
          for (var index = 0; index < 5; index += 1)
            XFile.fromData(_pngBytes, name: 'photo-$index.png'),
        ],
        pickGalleryVideo: () async => null,
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

      await _openSystemPhotoPicker(tester);
      expect(requestsStarted, 0);
      expect(peakActiveRequests, 0);
    });

    testWidgets('numbers recent photo selection and returns to the menu', (
      tester,
    ) async {
      final photoLibrary = _FakePhotoLibrary([
        RecentPhoto(id: 'one', thumbnailBytes: _gifBytes),
        RecentPhoto(id: 'two', thumbnailBytes: _gifBytes),
        RecentPhoto(id: 'three', thumbnailBytes: _gifBytes),
      ]);

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          photoLibrary: photoLibrary,
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      final compactComposerWidth = tester
          .getSize(find.byKey(const ValueKey('composer-width-transition')))
          .width;

      await _openAttachmentMenu(tester);
      await tester.tap(find.text('Photos'));
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('photo-gallery-picker')),
        findsOneWidget,
      );
      expect(find.byTooltip('Back to attachment options'), findsWidgets);
      expect(find.text('All photos'), findsOneWidget);
      expect(
        tester
            .getSize(find.byKey(const ValueKey('attachment-surface-popover')))
            .width,
        closeTo(compactComposerWidth / 0.85, 0.5),
      );

      await tester.tap(find.byKey(const ValueKey('recent-photo-two')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('recent-photo-one')));
      await tester.pumpAndSettle();

      expect(find.text('Add 2 photos'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('photo-selection-index-two')),
          matching: find.text('1'),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('photo-selection-index-one')),
          matching: find.text('2'),
        ),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const ValueKey('recent-photo-two')));
      await tester.pumpAndSettle();

      expect(find.text('Add 1 photo'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('photo-selection-index-one')),
          matching: find.text('1'),
        ),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const ValueKey('photo-gallery-back')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('photo-gallery-picker')), findsNothing);
      expect(
        find.byKey(const ValueKey('attachment-trigger-menu')).hitTestable(),
        findsOneWidget,
      );
      expect(find.text('Camera'), findsOneWidget);
      expect(find.text('Photos'), findsOneWidget);
    });

    testWidgets('attachment menu uses roomy rows and surrounding padding', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await _openAttachmentMenu(tester);

      final menu = find.byKey(const ValueKey('attachment-menu'));
      final surface = find.byKey(const ValueKey('attachment-surface-popover'));
      final rows = [
        for (final label in ['camera', 'photos', 'video', 'files'])
          find.byKey(ValueKey('attachment-menu-item-$label')),
      ];
      final menuRect = tester.getRect(menu);

      final material = tester.widget<Material>(surface);
      final shape = material.shape! as RoundedRectangleBorder;
      expect(shape.borderRadius, BorderRadius.circular(Radii.popover));
      expect(shape.side.color, Colors.black.withValues(alpha: 0.04));
      expect(material.elevation, appPopoverElevation);
      expect(
        material.shadowColor,
        appPopoverShadowColor(tester.element(surface)),
      );
      expect(menuRect.size, const Size(216, 264));
      for (final row in rows) {
        expect(tester.getSize(row).height, 52);
        expect(tester.getRect(row).left - menuRect.left, Grid.xs);
        expect(menuRect.right - tester.getRect(row).right, Grid.xs);
      }
      for (final label in ['Camera', 'Photos', 'Video', 'Files']) {
        final text = tester.widget<Text>(find.text(label));
        expect(text.style?.fontSize, 20);
        expect(text.style?.fontFamily, 'Inter');
      }
      final icons = [
        for (final label in ['camera', 'photos', 'video', 'files'])
          find.byKey(ValueKey('attachment-menu-icon-$label')),
      ];
      final labels = [
        for (final label in ['camera', 'photos', 'video', 'files'])
          find.byKey(ValueKey('attachment-menu-label-$label')),
      ];
      for (final icon in icons) {
        expect(tester.getSize(icon).width, 28);
        expect(
          tester
              .widget<Icon>(
                find.descendant(of: icon, matching: find.byType(Icon)),
              )
              .size,
          24,
        );
      }
      final labelLeft = tester.getRect(labels.first).left;
      for (var index = 0; index < labels.length; index += 1) {
        expect(tester.getRect(labels[index]).left, labelLeft);
        expect(
          tester.getRect(labels[index]).center.dy,
          tester.getRect(rows[index]).center.dy,
        );
      }
      expect(tester.getRect(rows.first).top - menuRect.top, Grid.xs);
      expect(menuRect.bottom - tester.getRect(rows.last).bottom, Grid.xs);
      for (var index = 1; index < rows.length; index += 1) {
        expect(
          tester.getRect(rows[index]).top -
              tester.getRect(rows[index - 1]).bottom,
          Grid.xxs,
        );
      }
    });

    testWidgets('tapping outside dismisses the Android attachment menu', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      try {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(nostr.Keys.generate().nsec),
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await _openAttachmentMenu(tester);
        expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
        expect(
          find.byKey(const ValueKey('attachment-dismiss-barrier')),
          findsOneWidget,
        );

        await tester.tapAt(const Offset(24, 24));
        await tester.pumpAndSettle();

        expect(find.byKey(const ValueKey('attachment-menu')), findsNothing);
        expect(
          find.byKey(const ValueKey('attachment-dismiss-barrier')),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('attachment-trigger-closed')).hitTestable(),
          findsOneWidget,
        );
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets(
      'attachment menu grows rows and scrolls for accessibility text',
      (tester) async {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(nostr.Keys.generate().nsec),
            textScaler: const TextScaler.linear(4),
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await _openAttachmentMenu(tester);

        final menu = find.byKey(const ValueKey('attachment-menu'));
        final rows = [
          for (final label in ['camera', 'photos', 'video', 'files'])
            find.byKey(ValueKey('attachment-menu-item-$label')),
        ];
        final scrollView = tester.widget<ListView>(
          find.byKey(const ValueKey('attachment-menu-scroll')),
        );

        expect(tester.getSize(menu), const Size(216, 372));
        expect(tester.getSize(rows.first).height, greaterThan(52));
        expect(scrollView.physics, isA<AlwaysScrollableScrollPhysics>());
        await tester.drag(
          find.byKey(const ValueKey('attachment-menu-scroll')),
          const Offset(0, -300),
        );
        await tester.pump();
        expect(tester.getSize(rows.last).height, greaterThan(52));
        expect(tester.takeException(), isNull);
      },
    );

    testWidgets('defers camera startup until the surface morph finishes', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      try {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(nostr.Keys.generate().nsec),
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );
        final compactComposerWidth = tester
            .getSize(find.byKey(const ValueKey('composer-width-transition')))
            .width;

        await _openAttachmentMenu(tester);
        await tester.tap(find.text('Camera'));
        await tester.pump();

        expect(
          find.byKey(const ValueKey('camera-initialization-deferred')),
          findsOneWidget,
        );

        await tester.pump(const Duration(milliseconds: 300));
        expect(
          find.byKey(const ValueKey('camera-initialization-deferred')),
          findsOneWidget,
        );

        await tester.pump(const Duration(milliseconds: 20));
        expect(
          find.byKey(const ValueKey('camera-initialization-ready')),
          findsOneWidget,
        );
        expect(
          tester
              .getSize(find.byKey(const ValueKey('attachment-surface-popover')))
              .width,
          closeTo(compactComposerWidth / 0.85, 0.5),
        );
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('photo picker errors keep the action visible at large text', (
      tester,
    ) async {
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async =>
            throw PlatformException(code: 'photo_picker_failed'),
        pickGalleryVideo: () async => null,
      );

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          textScaler: const TextScaler.linear(1.2),
          photoLibrary: _FakePhotoLibrary([
            RecentPhoto(id: 'one', thumbnailBytes: _gifBytes),
            RecentPhoto(id: 'two', thumbnailBytes: _gifBytes),
            RecentPhoto(id: 'three', thumbnailBytes: _gifBytes),
            RecentPhoto(id: 'four', thumbnailBytes: _gifBytes),
          ]),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      expect(find.byKey(const ValueKey('photo-gallery-error')), findsOneWidget);
      expect(
        find.byKey(const ValueKey('photo-gallery-action')).hitTestable(),
        findsOneWidget,
      );
    });

    testWidgets(
      'shows post-send upload progress above the composer and cancels',
      (tester) async {
        final uploadResponse = Completer<http.Response>();
        var sent = false;
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
          httpClient: http_testing.MockClient(
            (request) => uploadResponse.future,
          ),
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
          pickGalleryImages: () async => [
            XFile.fromData(_pngBytes, name: 'tiny.png'),
          ],
        );

        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: uploadService,
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {
                  sent = true;
                },
          ),
        );

        await _openSystemPhotoPicker(tester);
        await tester.pump();

        expect(
          find.byKey(const ValueKey('compose-upload-progress')),
          findsNothing,
        );

        await _expandComposer(tester);
        await tester.tap(find.byIcon(LucideIcons.arrowUp));
        await tester.pump();

        expect(
          find.byKey(const ValueKey('compose-upload-progress')),
          findsOneWidget,
        );
        expect(find.text('Uploading'), findsOneWidget);
        expect(find.text('100%'), findsOneWidget);
        expect(
          find.byKey(const ValueKey('compose-upload-cancel')),
          findsOneWidget,
        );
        expect(
          tester
              .widget<AnimatedFractionallySizedBox>(
                find.byKey(const ValueKey('compose-upload-progress-fill')),
              )
              .widthFactor,
          1,
        );
        final progressFill = tester.widget<ColoredBox>(
          find.descendant(
            of: find.byKey(const ValueKey('compose-upload-progress-fill')),
            matching: find.byType(ColoredBox),
          ),
        );
        expect(progressFill.color.a, closeTo(0.12, 0.001));
        expect(
          tester
              .widget<AnimatedFractionallySizedBox>(
                find.byKey(const ValueKey('compose-upload-progress-fill')),
              )
              .heightFactor,
          1,
        );
        expect(
          tester
              .widget<Padding>(
                find.byKey(const ValueKey('compose-upload-cancel-padding')),
              )
              .padding,
          const EdgeInsets.symmetric(
            horizontal: Grid.half,
            vertical: Grid.quarter,
          ),
        );

        await tester.pump(const Duration(milliseconds: 220));
        await tester.tap(find.byKey(const ValueKey('compose-upload-cancel')));
        await tester.pump();
        expect(
          find.byKey(const ValueKey('compose-upload-progress')),
          findsOneWidget,
        );
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('compose-upload-progress')),
          findsNothing,
        );

        uploadResponse.complete(
          http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/test.png',
              'sha256':
                  '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('compose-upload-progress')),
          findsNothing,
        );
        expect(sent, isFalse);
      },
    );

    testWidgets('surfaces an error and keeps the draft when a community switch '
        'cancels a text-only send', (tester) async {
      final agentPubkey = 'c' * 64;
      final signer = nostr.Keys.generate();
      final publishedEvents = <Map<String, dynamic>>[];
      var sendCount = 0;

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(signer.nsec),
          currentPubkey: signer.public,
          relayAgents: [_testAgent(agentPubkey)],
          channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
          relayConfig: () => _SwitchableRelayConfigNotifier(
            RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
          ),
          onSend: (_, _, {mediaTags = const <List<String>>[]}) async {
            sendCount += 1;
          },
        ),
      );

      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      final session = container.read(relaySessionProvider.notifier);
      // Switch community the moment the agent's kind:9000 add is
      // acknowledged. That is the only window in which the guard can fire:
      // everything before the first relay round trip runs in the same
      // microtask as the ChannelActions read.
      session.debugAttachSocketForTest(
        _RecordingRelaySocket(
          publishedEvents,
          session.debugHandleSocketMessageForTest,
          onEventAcknowledged: (event) {
            if (event['kind'] != 9000) return;
            container
                .read(relayConfigProvider.notifier)
                .update(baseUrl: 'https://other.example', nsec: signer.nsec);
          },
        ),
      );

      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@hel');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Helper Bot'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();

      // The cancelled send must not reach the relay.
      expect(sendCount, 0);
      // The escaped StateError itself is pinned by flutter_test's own
      // unhandled-exception reporting, which fails this test if the error
      // is not caught. Deliberately no `takeException()` row: the framework
      // has already consumed the error by this point, so such a row reads
      // null whether or not the error escaped, and would never fail.
      //
      // What needs pinning is the user-visible half. The composer's own
      // error line cannot carry it, because the same identity change resets
      // that state on the very next frame, so the surface must outlive the
      // switch.
      expect(
        find.widgetWithText(
          SnackBar,
          'Message not sent: the community changed',
        ),
        findsOneWidget,
      );
      // Characterization, not a fix: the unsent text is already retained in
      // the originating community's own draft store (the send path never
      // reaches `clearComposer`, and the persist listener wrote it while
      // typing), so it is waiting when the user switches back. This row
      // holds without the catch above and exists to keep it that way.
      final storedDrafts = _testPrefs.getString(
        'compose_drafts_v1:https://relay.example:${signer.public}',
      );
      expect(storedDrafts, isNotNull);
      expect(
        (jsonDecode(storedDrafts!) as List)
            .map((d) => (d as Map<String, dynamic>)['text'])
            .toList(),
        contains('hello @Helper Bot'),
      );
    });

    testWidgets(
      'stale channel actions cannot invite after a community switch',
      (tester) async {
        final signer = nostr.Keys.generate();
        final publishedEvents = <Map<String, dynamic>>[];
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(signer.nsec),
            currentPubkey: signer.public,
            relayConfig: () => _SwitchableRelayConfigNotifier(
              RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
            ),
            onSend: (_, _, {mediaTags = const <List<String>>[]}) async {},
          ),
        );

        final container = ProviderScope.containerOf(
          tester.element(find.byType(ComposeBar)),
        );
        final session = container.read(relaySessionProvider.notifier);
        session.debugAttachSocketForTest(
          _RecordingRelaySocket(
            publishedEvents,
            session.debugHandleSocketMessageForTest,
          ),
        );
        final staleActions = container.read(channelActionsProvider);

        container
            .read(relayConfigProvider.notifier)
            .update(baseUrl: 'https://other-community.example', nsec: null);
        await tester.pump();

        await expectLater(
          staleActions.addMembers(
            channelId: 'channel-1',
            pubkeys: ['c' * 64],
            role: 'bot',
          ),
          throwsA(isA<StateError>()),
        );
        expect(publishedEvents, isEmpty);
      },
    );

    testWidgets(
      'does not add a mentioned non-member when media upload is cancelled',
      (tester) async {
        final agentPubkey = 'c' * 64;
        final signer = nostr.Keys.generate();
        final publishedEvents = <Map<String, dynamic>>[];
        final uploadResponse = Completer<http.Response>();
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: signer.nsec,
          httpClient: http_testing.MockClient(
            (request) => uploadResponse.future,
          ),
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
          pickGalleryImages: () async => [
            XFile.fromData(_pngBytes, name: 'tiny.png'),
          ],
        );

        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: uploadService,
            currentPubkey: signer.public,
            relayAgents: [_testAgent(agentPubkey)],
            channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
            onSend: (_, _, {mediaTags = const <List<String>>[]}) async {},
          ),
        );

        final container = ProviderScope.containerOf(
          tester.element(find.byType(ComposeBar)),
        );
        final session = container.read(relaySessionProvider.notifier);
        final socket = _RecordingRelaySocket(
          publishedEvents,
          session.debugHandleSocketMessageForTest,
        );
        session.debugAttachSocketForTest(socket);

        await _openSystemPhotoPicker(tester);
        await tester.pumpAndSettle();
        await _expandComposer(tester);
        await tester.enterText(find.byType(TextField), '@hel');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Helper Bot'));
        await tester.pumpAndSettle();
        await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
        await tester.tap(find.byIcon(LucideIcons.arrowUp));
        await tester.pump();

        expect(
          publishedEvents.where((event) => event['kind'] == 9000),
          isEmpty,
        );

        await tester.pump(const Duration(milliseconds: 220));
        await tester.tap(find.byKey(const ValueKey('compose-upload-cancel')));
        uploadResponse.complete(
          http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/test.png',
              'sha256':
                  '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          ),
        );
        await tester.pumpAndSettle();

        expect(
          publishedEvents.where((event) => event['kind'] == 9000),
          isEmpty,
        );
      },
    );

    testWidgets('renders markdown formatting without visible delimiters', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await _expandComposer(tester);
      await tester.enterText(
        find.byType(TextField),
        '**bold** _italic_ ~~strike~~ `code`',
      );

      final textField = tester.widget<TextField>(find.byType(TextField));
      final textSpan = textField.controller!.buildTextSpan(
        context: tester.element(find.byType(TextField)),
        style: tester.element(find.byType(TextField)).textTheme.bodyLarge,
        withComposing: false,
      );
      final spans = _flattenStyledTextSpans(textSpan);

      expect(textSpan.toPlainText(), '**bold** _italic_ ~~strike~~ `code`');
      expect(
        spans.singleWhere((span) => span.text == 'bold').style.fontWeight,
        FontWeight.w700,
      );
      expect(
        spans.singleWhere((span) => span.text == 'italic').style.fontStyle,
        FontStyle.italic,
      );
      expect(
        spans.singleWhere((span) => span.text == 'strike').style.decoration,
        TextDecoration.lineThrough,
      );
      expect(
        spans.singleWhere((span) => span.text == 'code').style.fontFamily,
        'GeistMono',
      );
      expect(
        spans
            .where((span) => {'**', '_', '~~', '`'}.contains(span.text))
            .every((span) => (span.style.fontSize ?? 1) < 1),
        isTrue,
      );

      textField.controller!.value = textField.controller!.value.copyWith(
        composing: const TextRange(start: 2, end: 6),
      );
      final composingSpan = textField.controller!.buildTextSpan(
        context: tester.element(find.byType(TextField)),
        style: tester.element(find.byType(TextField)).textTheme.bodyLarge,
        withComposing: true,
      );
      final composingSpans = _flattenStyledTextSpans(composingSpan);
      expect(
        composingSpans
            .where((span) => span.text == '**')
            .every((span) => (span.style.fontSize ?? 1) < 1),
        isTrue,
      );
      expect(
        composingSpans
            .singleWhere((span) => span.text == 'bold')
            .style
            .decoration,
        TextDecoration.underline,
      );
    });

    testWidgets('uses the primary color for formatting actions', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );

      await _expandComposer(tester);
      await tester.tap(find.byIcon(LucideIcons.aLargeSmall));
      await tester.pumpAndSettle();

      final boldFinder = find.byIcon(LucideIcons.bold);
      final boldIcon = tester.widget<Icon>(boldFinder);
      final colors = tester.element(boldFinder).colors;
      expect(boldIcon.color, colors.primary);
    });

    testWidgets('pasted image follows the attachment preview and send path', (
      tester,
    ) async {
      final keychain = nostr.Keys.generate();
      var galleryPickerCalled = false;
      Uint8List? uploadedBytes;
      String? uploadedMimeType;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: keychain.nsec,
        httpClient: http_testing.MockClient((request) async {
          uploadedBytes = request.bodyBytes;
          uploadedMimeType = request.headers['Content-Type'];
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/pasted.png',
              'sha256':
                  '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
              'thumb': 'https://relay.example/media/pasted.thumb.jpg',
            }),
            200,
          );
        }),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async {
          galleryPickerCalled = true;
          return null;
        },
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

      await _expandComposer(tester);
      final textField = tester.widget<TextField>(find.byType(TextField));
      final insertionConfiguration = textField.contentInsertionConfiguration;
      expect(insertionConfiguration, isNotNull);
      expect(
        insertionConfiguration!.allowedMimeTypes,
        containsAll(['image/jpeg', 'image/png', 'image/webp']),
      );

      insertionConfiguration.onContentInserted(
        KeyboardInsertedContent(
          mimeType: 'image/png',
          uri: 'content://clipboard/pasted.png',
          data: _pngBytes,
        ),
      );
      await tester.pumpAndSettle();

      expect(galleryPickerCalled, isFalse);
      expect(uploadedBytes, isNull);
      expect(find.byTooltip('Remove attachment'), findsOneWidget);
      expect(
        find.byWidgetPredicate(
          (widget) => widget is Image && widget.image is MemoryImage,
        ),
        findsOneWidget,
      );

      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();

      expect(uploadedBytes, _pngBytes);
      expect(uploadedMimeType, 'image/png');
      expect(sentContent, '\n![image](https://relay.example/media/pasted.png)');
      expect(sentMediaTags, hasLength(1));
      expect(
        sentMediaTags.single,
        contains('url https://relay.example/media/pasted.png'),
      );
    });

    testWidgets('iOS native context menu preserves defaults and pastes image', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
          httpClient: http_testing.MockClient(
            (request) async => http.Response(
              jsonEncode({
                'url': 'https://relay.example/media/ios-native-paste.png',
                'sha256':
                    '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                'size': 16,
                'type': 'image/png',
                'uploaded': 1,
              }),
              200,
            ),
          ),
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
          readClipboardImage: () async => _pngBytes,
        );
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: uploadService,
            supportsShowingSystemContextMenu: true,
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await _expandComposer(tester);
        final textField = tester.widget<TextField>(find.byType(TextField));
        final editableTextState = tester.state<EditableTextState>(
          find.byType(EditableText),
        );
        final defaultItems = SystemContextMenu.getDefaultItems(
          editableTextState,
        );
        final menu =
            textField.contextMenuBuilder!(
                  tester.element(find.byType(TextField)),
                  editableTextState,
                )
                as SystemContextMenu;
        final pasteImage = menu.items.first as IOSSystemContextMenuItemCustom;

        expect(pasteImage.title, 'Paste Image');
        expect(menu.items.skip(1), orderedEquals(defaultItems));
        pasteImage.onPressed();
        await tester.pumpAndSettle();

        expect(find.byTooltip('Remove attachment'), findsOneWidget);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('iOS hides Paste Image when clipboard has no image', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      _setMockMediaUploadPlatformHandler((call) async {
        if (call.method == 'clipboardHasImage') return false;
        return null;
      });
      try {
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
        );
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: uploadService,
            supportsShowingSystemContextMenu: true,
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );
        await tester.pump();

        await _expandComposer(tester);
        final textField = tester.widget<TextField>(find.byType(TextField));
        final editableTextState = tester.state<EditableTextState>(
          find.byType(EditableText),
        );
        final menu =
            textField.contextMenuBuilder!(
                  tester.element(find.byType(TextField)),
                  editableTextState,
                )
                as SystemContextMenu;

        expect(menu.items.whereType<IOSSystemContextMenuItemCustom>(), isEmpty);
      } finally {
        _setMockMediaUploadPlatformHandler((call) async {
          switch (call.method) {
            case 'sanitizeImageForUpload':
              final arguments = call.arguments as Map<Object?, Object?>;
              return arguments['bytes'] as Uint8List;
            case 'transcodeImageToJpeg':
              return _pngBytes;
            case 'clipboardHasImage':
              return true;
            default:
              return null;
          }
        });
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('iOS adaptive Paste Image reads the clipboard into shared path', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
          httpClient: http_testing.MockClient(
            (request) async => http.Response(
              jsonEncode({
                'url': 'https://relay.example/media/ios-paste.png',
                'sha256':
                    '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                'size': 16,
                'type': 'image/png',
                'uploaded': 1,
              }),
              200,
            ),
          ),
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
          readClipboardImage: () async => _pngBytes,
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

        await _expandComposer(tester);
        final textField = tester.widget<TextField>(find.byType(TextField));
        final editableTextState = tester.state<EditableTextState>(
          find.byType(EditableText),
        );
        final menu =
            textField.contextMenuBuilder!(
                  tester.element(find.byType(TextField)),
                  editableTextState,
                )
                as AdaptiveTextSelectionToolbar;
        final pasteImage = menu.buttonItems!.singleWhere(
          (item) => item.label == 'Paste Image',
        );
        pasteImage.onPressed!();
        await tester.pumpAndSettle();

        expect(find.byTooltip('Remove attachment'), findsOneWidget);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('shows an error when pasted image bytes are unavailable', (
      tester,
    ) async {
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
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

      await _expandComposer(tester);
      final textField = tester.widget<TextField>(find.byType(TextField));
      textField.contentInsertionConfiguration!.onContentInserted(
        const KeyboardInsertedContent(
          mimeType: 'image/png',
          uri: 'content://clipboard/unavailable.png',
        ),
      );
      await tester.pump();

      expect(find.text('Unable to read pasted image'), findsOneWidget);
      expect(find.byTooltip('Remove attachment'), findsNothing);
    });

    testWidgets('iOS Paste Image reports an unavailable clipboard image', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
          readClipboardImage: () async => null,
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

        await _expandComposer(tester);
        final textField = tester.widget<TextField>(find.byType(TextField));
        final editableTextState = tester.state<EditableTextState>(
          find.byType(EditableText),
        );
        final menu =
            textField.contextMenuBuilder!(
                  tester.element(find.byType(TextField)),
                  editableTextState,
                )
                as AdaptiveTextSelectionToolbar;
        menu.buttonItems!
            .singleWhere((item) => item.label == 'Paste Image')
            .onPressed!();
        await tester.pumpAndSettle();

        expect(find.text('Unable to read pasted image'), findsOneWidget);
        expect(find.byTooltip('Remove attachment'), findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('does not add Paste Image to non-iOS context menus', (
      tester,
    ) async {
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
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

      await _expandComposer(tester);
      final textField = tester.widget<TextField>(find.byType(TextField));
      final editableTextState = tester.state<EditableTextState>(
        find.byType(EditableText),
      );
      final menu =
          textField.contextMenuBuilder!(
                tester.element(find.byType(TextField)),
                editableTextState,
              )
              as AdaptiveTextSelectionToolbar;

      expect(
        menu.buttonItems!.where((item) => item.label == 'Paste Image'),
        isEmpty,
      );
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

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();

      final removeButtonFinder = find.byTooltip('Remove attachment');

      expect(removeButtonFinder, findsOneWidget);

      final attachmentTopRight = tester.getTopRight(
        find
            .ancestor(of: removeButtonFinder, matching: find.byType(Container))
            .first,
      );
      final attachmentTopLeft = tester.getTopLeft(
        find
            .ancestor(of: removeButtonFinder, matching: find.byType(Container))
            .first,
      );
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

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), 'Keep this draft');
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();

      expect(find.textContaining('upload failed'), findsOneWidget);
      expect(find.text('Keep this draft'), findsOneWidget);
      expect(find.byTooltip('Remove attachment'), findsOneWidget);
    });

    for (final statusCode in [
      HttpStatus.unsupportedMediaType,
      HttpStatus.unprocessableEntity,
    ]) {
      testWidgets('shows friendly copy for a $statusCode upload response', (
        tester,
      ) async {
        final keychain = nostr.Keys.generate();
        final uploadService = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: keychain.nsec,
          httpClient: http_testing.MockClient(
            (request) async => http.Response(
              '{"error":"media contains metadata or a non-canonical metadata channel"}',
              statusCode,
            ),
          ),
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

        await _openSystemPhotoPicker(tester);
        await tester.pumpAndSettle();
        await _expandComposer(tester);
        await tester.tap(find.byIcon(LucideIcons.arrowUp));
        await tester.pumpAndSettle();

        expect(
          find.text("We couldn't prepare this image for upload."),
          findsOneWidget,
        );
        expect(find.textContaining('media contains metadata'), findsNothing);
        expect(find.textContaining('$statusCode'), findsNothing);
      });
    }

    testWidgets('adds a sanitized GIF attachment', (tester) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient((request) async {
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/animated.gif',
              'sha256':
                  '4444444444444444444444444444444444444444444444444444444444444444',
              'size': request.bodyBytes.length,
              'type': 'image/gif',
              'uploaded': 1,
            }),
            200,
          );
        }),
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

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Remove attachment'), findsOneWidget);
    });

    testWidgets('adds a selected non-member agent as a bot before sending', (
      tester,
    ) async {
      final agentPubkey = 'c' * 64;
      final signer = nostr.Keys.generate();
      final publishedEvents = <Map<String, dynamic>>[];
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: signer.nsec,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );
      String? sentContent;
      List<String> sentMentionPubkeys = const <String>[];

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: uploadService,
          currentPubkey: signer.public,
          relayAgents: [
            AgentDirectoryEntry(
              pubkey: agentPubkey,
              displayName: 'Helper Bot',
              respondTo: 'anyone',
              channelIds: const ['shared-channel'],
            ),
          ],
          channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {
                sentContent = content;
                sentMentionPubkeys = mentionPubkeys;
              },
        ),
      );

      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      final session = container.read(relaySessionProvider.notifier);
      final socket = _RecordingRelaySocket(
        publishedEvents,
        session.debugHandleSocketMessageForTest,
      );
      session.debugAttachSocketForTest(socket);

      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@hel');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Helper Bot'));
      await tester.pumpAndSettle();
      expect(find.byIcon(LucideIcons.bot), findsOneWidget);
      expect(
        find.byKey(const ValueKey('composer-agent-mention-chip')),
        findsOneWidget,
      );
      await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();

      expect(sentContent, 'hello @Helper Bot');
      expect(sentMentionPubkeys, [agentPubkey]);
      final addMemberEvent = publishedEvents.singleWhere(
        (event) => event['kind'] == 9000,
      );
      expect(addMemberEvent['tags'], [
        ['h', 'channel-1'],
        ['p', agentPubkey],
        ['role', 'bot'],
      ]);
    });

    testWidgets(
      'renders chips only for selected agents outside code and composition',
      (tester) async {
        final semantics = tester.ensureSemantics();
        final signer = nostr.Keys.generate();
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(signer.nsec),
            currentPubkey: signer.public,
            relayAgents: [_testAgent('f' * 64)],
            channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {},
          ),
        );

        await _expandComposer(tester);
        await tester.enterText(find.byType(TextField), '@Helper Bot');
        await tester.pump();
        expect(
          find.byKey(const ValueKey('composer-agent-mention-chip')),
          findsNothing,
        );

        await tester.enterText(find.byType(TextField), '@hel');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Helper Bot'));
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('composer-agent-mention-chip')),
          findsOneWidget,
        );
        expect(
          find.bySemanticsLabel('Agent mention: Helper Bot'),
          findsOneWidget,
        );
        expect(find.bySemanticsLabel('Helper Bot'), findsNothing);

        await tester.enterText(find.byType(TextField), '`@Helper Bot`');
        await tester.pump();
        expect(
          find.byKey(const ValueKey('composer-agent-mention-chip')),
          findsNothing,
        );

        await tester.enterText(find.byType(TextField), '@Helper Bot typing');
        final textField = tester.widget<TextField>(find.byType(TextField));
        textField.controller!.value = textField.controller!.value.copyWith(
          composing: const TextRange(start: 12, end: 18),
        );
        await tester.pump();
        expect(
          find.byKey(const ValueKey('composer-agent-mention-chip')),
          findsOneWidget,
        );
        await tester.pump(const Duration(milliseconds: 250));
        semantics.dispose();
      },
    );

    testWidgets('does not mutate a DM when mentioning a non-member agent', (
      tester,
    ) async {
      final agentPubkey = 'd' * 64;
      final signer = nostr.Keys.generate();
      final publishedEvents = <Map<String, dynamic>>[];
      String? sentContent;

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(signer.nsec),
          currentPubkey: signer.public,
          relayAgents: [_testAgent(agentPubkey)],
          channels: [
            _makeCurrentChannel(channelType: 'dm'),
            _makeSharedMemberChannel(),
          ],
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

      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      final session = container.read(relaySessionProvider.notifier);
      final socket = _RecordingRelaySocket(
        publishedEvents,
        session.debugHandleSocketMessageForTest,
      );
      session.debugAttachSocketForTest(socket);

      await _selectAndSendAgentMention(tester);

      expect(sentContent, 'hello @Helper Bot');
      expect(publishedEvents.where((event) => event['kind'] == 9000), isEmpty);
    });

    testWidgets('waits for current member data before adding an agent', (
      tester,
    ) async {
      final agentPubkey = 'e' * 64;
      final signer = nostr.Keys.generate();
      final membersCompleter = Completer<List<ChannelMember>>();
      final publishedEvents = <Map<String, dynamic>>[];
      var didSend = false;

      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(signer.nsec),
          currentPubkey: signer.public,
          membersFuture: membersCompleter.future,
          relayAgents: [_testAgent(agentPubkey)],
          channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
          onSend:
              (
                content,
                mentionPubkeys, {
                mediaTags = const <List<String>>[],
              }) async {
                didSend = true;
              },
        ),
      );

      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      final session = container.read(relaySessionProvider.notifier);
      final socket = _RecordingRelaySocket(
        publishedEvents,
        session.debugHandleSocketMessageForTest,
      );
      session.debugAttachSocketForTest(socket);

      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@hel');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Helper Bot'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pump();

      expect(didSend, isFalse);
      expect(publishedEvents.where((event) => event['kind'] == 9000), isEmpty);

      membersCompleter.complete([
        ChannelMember(
          pubkey: agentPubkey,
          role: 'admin',
          joinedAt: DateTime(2024),
        ),
      ]);
      await tester.pumpAndSettle();

      expect(didSend, isTrue);
      expect(publishedEvents.where((event) => event['kind'] == 9000), isEmpty);
    });

    testWidgets(
      'skips the agent add in a private channel when not owner/admin',
      (tester) async {
        final agentPubkey = 'a' * 64;
        final signer = nostr.Keys.generate();
        final publishedEvents = <Map<String, dynamic>>[];
        var didSend = false;
        List<String> sentMentionPubkeys = const <String>[];
        List<List<String>> sentMediaTags = const <List<String>>[];

        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(signer.nsec),
            currentPubkey: signer.public,
            // Plain member of a private channel: the relay rejects any add, so
            // the composer must not attempt one — and must still send.
            members: [
              ChannelMember(
                pubkey: signer.public,
                role: 'member',
                joinedAt: DateTime(2024),
              ),
            ],
            relayAgents: [_testAgent(agentPubkey)],
            channels: [
              _makeCurrentChannel(visibility: 'private'),
              _makeSharedMemberChannel(),
            ],
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {
                  didSend = true;
                  sentMentionPubkeys = mentionPubkeys;
                  sentMediaTags = mediaTags;
                },
          ),
        );

        final container = ProviderScope.containerOf(
          tester.element(find.byType(ComposeBar)),
        );
        final session = container.read(relaySessionProvider.notifier);
        final socket = _RecordingRelaySocket(
          publishedEvents,
          session.debugHandleSocketMessageForTest,
        );
        session.debugAttachSocketForTest(socket);

        await _expandComposer(tester);
        await tester.enterText(find.byType(TextField), '@hel');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Helper Bot'));
        await tester.pumpAndSettle();
        await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
        await tester.tap(find.byIcon(LucideIcons.arrowUp));
        await tester.pumpAndSettle();

        expect(didSend, isTrue);
        expect(
          publishedEvents.where((event) => event['kind'] == 9000),
          isEmpty,
        );
        // The un-added agent is demoted from p-tag to a reference mention.
        expect(sentMentionPubkeys, isEmpty);
        expect(
          sentMediaTags,
          contains(orderedEquals(['mention', agentPubkey])),
        );
        expect(find.text(privateChannelAddDeniedMessage), findsOneWidget);
      },
    );

    testWidgets('adds a sanitized animated PNG attachment', (tester) async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final uploadService = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient((request) async {
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/animated.png',
              'sha256':
                  '5555555555555555555555555555555555555555555555555555555555555555',
              'size': request.bodyBytes.length,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          );
        }),
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

      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Remove attachment'), findsOneWidget);
    });

    testWidgets('taps Video in chooser sheet and uploads video', (
      tester,
    ) async {
      final pickedVideo = XFile.fromData(
        Uint8List.fromList([0, 1, 2, 3]),
        mimeType: 'video/mp4',
        name: 'clip.mp4',
      );
      final uploadService = _FakeVideoUploadService(pickedVideo);

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

      await _openAttachmentMenu(tester);
      await tester.tap(find.text('Video'));
      await tester.pumpAndSettle();

      expect(find.byIcon(LucideIcons.video), findsOneWidget);

      final sendButton = find
          .ancestor(
            of: find.byIcon(LucideIcons.arrowUp),
            matching: find.byType(IconButton),
          )
          .hitTestable();
      expect(sendButton, findsOneWidget);
      await tester.tap(sendButton);
      await tester.pumpAndSettle();

      expect(uploadService.uploadedVideo, same(pickedVideo));
      expect(sentContent, '\n![video](https://relay.example/media/test.mp4)');
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

MediaUploadService _testUploadService(String nsec) {
  return MediaUploadService(
    baseUrl: 'https://relay.example',
    nsec: nsec,
    pickGalleryImage: () async => null,
    pickGalleryVideo: () async => null,
  );
}

AgentDirectoryEntry _testAgent(String pubkey) {
  return AgentDirectoryEntry(
    pubkey: pubkey,
    displayName: 'Helper Bot',
    respondTo: 'anyone',
    channelIds: const ['shared-channel'],
  );
}

Future<void> _expandComposer(WidgetTester tester) async {
  if (find.byType(TextField).evaluate().isNotEmpty) return;
  await tester.tap(find.text('Message\u2026'));
  await tester.pumpAndSettle();
}

Future<void> _openAttachmentMenu(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Add attachment').hitTestable());
  await tester.pumpAndSettle();
}

Future<void> _openSystemPhotoPicker(WidgetTester tester) async {
  await _openAttachmentMenu(tester);
  await tester.tap(find.text('Photos'));
  await tester.pumpAndSettle();
  await tester.tap(find.text('All photos'));
}

Future<void> _selectAndSendAgentMention(WidgetTester tester) async {
  await _expandComposer(tester);
  await tester.enterText(find.byType(TextField), '@hel');
  await tester.pumpAndSettle();
  await tester.tap(find.text('Helper Bot'));
  await tester.pumpAndSettle();
  await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
  await tester.tap(find.byIcon(LucideIcons.arrowUp));
  await tester.pumpAndSettle();
}

List<({String text, TextStyle style})> _flattenStyledTextSpans(
  InlineSpan root,
) {
  final result = <({String text, TextStyle style})>[];

  void visit(InlineSpan span, TextStyle inheritedStyle) {
    if (span is! TextSpan) return;
    final effectiveStyle = inheritedStyle.merge(span.style);
    if (span.text case final text?) {
      result.add((text: text, style: effectiveStyle));
    }
    for (final child in span.children ?? const <InlineSpan>[]) {
      visit(child, effectiveStyle);
    }
  }

  visit(root, const TextStyle());
  return result;
}

Channel _makeCurrentChannel({
  String channelType = 'stream',
  String visibility = 'open',
}) {
  return Channel(
    id: 'channel-1',
    name: 'current',
    channelType: channelType,
    visibility: visibility,
    description: '',
    createdBy: 'pubkey123',
    createdAt: DateTime(2024),
    memberCount: 2,
    isMember: true,
  );
}

Channel _makeSharedMemberChannel() {
  return Channel(
    id: 'shared-channel',
    name: 'shared',
    channelType: 'stream',
    visibility: 'open',
    description: '',
    createdBy: 'pubkey123',
    createdAt: DateTime(2024),
    memberCount: 5,
    isMember: true,
  );
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
