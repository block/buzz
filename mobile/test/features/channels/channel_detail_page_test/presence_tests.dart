part of '../channel_detail_page_test.dart';

void presenceTests() {
  testWidgets('DM header never substitutes offline for unknown presence', (
    tester,
  ) async {
    await _loadPresenceFonts(tester);
    final cache = _PresenceFixture();
    final channel = Channel(
      id: _channelId,
      name: 'DM',
      channelType: 'dm',
      description: 'Direct message',
      visibility: 'private',
      createdBy: 'self',
      createdAt: DateTime(2025),
      memberCount: 2,
      participants: const ['Self', 'Alice'],
      participantPubkeys: const ['self', 'alice'],
      isMember: true,
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        channel: channel,
        presenceCache: cache,
        users: const {
          'alice': UserProfile(
            pubkey: 'alice',
            displayName: 'Alice',
            ownerPubkey: 'self',
          ),
        },
      ),
    );
    await tester.pumpAndSettle();
    final avatar = find.byKey(const ValueKey('dm-header-avatar'));
    final bounds = tester.getRect(avatar);
    for (final entry in <String?, String>{
      null: 'Unknown',
      'online': 'Online',
      'away': 'Away',
      'offline': 'Offline',
    }.entries) {
      cache.setPresence(entry.key);
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('dm-header-presence')))
            .data,
        entry.value,
      );
      expect(
        tester.widget<MaskedAvatarBadge>(avatar).badge,
        entry.key == null ? isNull : isNotNull,
      );
      expect(tester.getRect(avatar), bounds);
      await _capturePresence(tester, 'dm-${entry.value}');
    }
    cache.setPresence(null);
    await tester.pumpAndSettle();
    expect(find.text('Offline'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('profile exposes one truthful presence label for every state', (
    tester,
  ) async {
    await _loadPresenceFonts(tester);
    final cache = _PresenceFixture();
    final semantics = tester.ensureSemantics();

    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        presenceCache: cache,
        users: const {
          'alice': UserProfile(
            pubkey: 'alice',
            displayName: 'Alice',
            ownerPubkey: 'self',
          ),
        },
        home: const Scaffold(body: UserProfileSheet(pubkey: 'alice')),
      ),
    );
    await tester.pumpAndSettle();
    for (final entry in <String?, String>{
      null: 'Unknown',
      'online': 'Online',
      'away': 'Away',
      'offline': 'Offline',
    }.entries) {
      cache.setPresence(entry.key);
      await tester.pumpAndSettle();
      expect(find.text(entry.value), findsOneWidget);
      expect(find.bySemanticsLabel('Presence: ${entry.value}'), findsOneWidget);
      if (entry.key != 'offline') expect(find.text('Offline'), findsNothing);
      await _capturePresence(tester, 'profile-${entry.value}');
    }
    expect(tester.takeException(), isNull);
    semantics.dispose();
  });
}

class _PresenceFixture extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {};
  @override
  void track(List<String> pubkeys) {}
  void setPresence(String? status) => state = {'alice': ?status};
}

// Opt-in rendered evidence; ordinary test runs perform no filesystem writes.
Future<void> _loadPresenceFonts(WidgetTester tester) async {
  if (Platform.environment['PRESENCE_SCREENSHOTS'] == null) return;
  await tester.binding.setSurfaceSize(const Size(390, 844));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await (FontLoader(
    'Inter',
  )..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'))).load();
}

Future<void> _capturePresence(WidgetTester tester, String name) async {
  final directory = Platform.environment['PRESENCE_SCREENSHOTS'];
  if (directory == null) return;
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byType(RepaintBoundary).first,
  );
  await tester.runAsync(() async {
    final image = await boundary.toImage();
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    await Directory(directory).create(recursive: true);
    await File('$directory/$name.png').writeAsBytes(data!.buffer.asUint8List());
    image.dispose();
  });
}
