import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/shared/widgets/navigation_backdrop.dart';
import 'package:buzz/features/profile/profile_sheet_layout.dart';
import 'package:buzz/shared/widgets/native_profile_sheet.dart';
import 'package:flutter/services.dart';
import 'package:buzz/shared/widgets/buzz_toast.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_directory.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/identity_names/identity_names_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _UserCache extends UserCacheNotifier {
  _UserCache(this._initial);

  final Map<String, UserProfile> _initial;

  @override
  Map<String, UserProfile> build() => _initial;

  @override
  UserProfile? get(String pubkey) => state[pubkey.toLowerCase()];

  @override
  Future<bool> preload(List<String> pubkeys) async => true;

  void replace(UserProfile profile) =>
      state = {...state, profile.pubkey: profile};
}

class _VerifiedProfiles extends VerifiedProfilesNotifier {
  @override
  Map<String, VerifiedProfile> build() => {};

  void replace(Map<String, VerifiedProfile> profiles) => state = profiles;
}

void main() {
  final scout = 'a' * 64;
  final other = 'b' * 64;

  for (final hasPubkey in [true, false]) {
    testWidgets('Led by replaces the profile (lead has pubkey: $hasPubkey)', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            userCacheProvider.overrideWith(
              () => _UserCache({
                scout: UserProfile(pubkey: scout, displayName: 'Scout'),
                other: UserProfile(pubkey: other, displayName: 'Lead'),
              }),
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showUserProfileSheet(context, scout),
                  child: const Text('Open Scout'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open Scout'));
      await tester.pumpAndSettle();
      final sheet = tester.widget<UserProfileSheet>(
        find.byType(UserProfileSheet),
      );
      sheet.onLead!((
        name: 'Lead',
        avatarUrl: null,
        pubkey: hasPubkey ? other : null,
      ));
      await tester.pumpAndSettle();
      expect(
        find.byType(ProfileSheetLayout, skipOffstage: false),
        findsOneWidget,
      );
      expect(find.text('Lead'), findsOneWidget);
      Navigator.of(tester.element(find.byType(ProfileSheetLayout))).pop();
      await tester.pumpAndSettle();
      expect(
        find.byType(ProfileSheetLayout, skipOffstage: false),
        findsNothing,
      );
      expect(find.text('Open Scout'), findsOneWidget);
    });
  }

  testWidgets(
    'copy feedback hides the native grabber until the last chip exits',
    (tester) async {
      const channel = MethodChannel('buzz/profile_sheet');
      final messenger = tester.binding.defaultBinaryMessenger;
      final visibility = <bool>[];
      messenger.setMockMethodCallHandler(channel, (call) async {
        if (call.method == 'height') {
          visibility.add(call.arguments['showGrabber'] as bool);
        }
        return null;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      final offset = ValueNotifier(0.0);
      addTearDown(offset.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: NativeProfileSheet(
              id: 1,
              expanded: true,
              offset: offset,
              child: const ProfileSheetLayout(
                hero: Text('Profile'),
                body: SizedBox(height: 900),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(visibility.last, isTrue);
      final context = tester.element(find.text('Profile'));
      showBuzzToast(context, 'Email copied');
      await tester.pumpAndSettle();
      expect(visibility.last, isFalse);
      await tester.pump(const Duration(seconds: 2));
      showBuzzToast(context, 'Public key copied');
      await tester.pumpAndSettle();
      await tester.pump(const Duration(seconds: 2));
      expect(visibility.last, isFalse);
      await tester.pump(const Duration(seconds: 3));
      await tester.pumpAndSettle();
      expect(visibility.last, isTrue);
    },
  );

  testWidgets(
    'rejected native presentation restores the host before fallback',
    (tester) async {
      const channel = MethodChannel('buzz/profile_sheet');
      final calls = <String>[];
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(channel, (call) async {
        calls.add(call.method);
        return call.method != 'present';
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => TextButton(
              onPressed: () => showNativeProfileSheet<void>(
                context: context,
                fallback: () async {
                  calls.add('fallback');
                },
                builder: (_) => const Text('Native profile'),
              ),
              child: const Text('Open'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      expect(calls, ['prepare', 'present', 'dismiss', 'fallback']);
      expect(find.text('Native profile'), findsNothing);
      expect(find.text('Open').hitTestable(), findsOneWidget);
    },
  );

  testWidgets('native profile follows appearance changes while open', (
    tester,
  ) async {
    const channel = MethodChannel('buzz/profile_sheet');
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(channel, (_) async => true);
    addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
    final platform = tester.binding.platformDispatcher;
    platform.platformBrightnessTestValue = Brightness.light;
    addTearDown(platform.clearPlatformBrightnessTestValue);
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        darkTheme: AppTheme.dark(),
        home: Theme(
          data: AppTheme.light(),
          child: Builder(
            builder: (context) => TextButton(
              onPressed: () => showNativeProfileSheet<void>(
                context: context,
                fallback: () async {},
                builder: (context) => Text(Theme.of(context).brightness.name),
              ),
              child: const Text('Open'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    expect(find.text('light'), findsOneWidget);
    platform.platformBrightnessTestValue = Brightness.dark;
    await tester.pumpAndSettle();
    expect(find.text('dark'), findsOneWidget);
    platform.platformBrightnessTestValue = Brightness.light;
    await tester.pumpAndSettle();
    expect(find.text('light'), findsOneWidget);
    Navigator.of(tester.element(find.text('light'))).pop();
    await tester.pumpAndSettle();
  });

  testWidgets('the enlarged portrait opens in the full native sheet', (
    tester,
  ) async {
    const bridge = MethodChannel('buzz/profile_sheet');
    final photoModes = <bool>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
      call,
    ) async {
      if (call.method == 'photo') {
        photoModes.add(call.arguments['open'] as bool);
      }
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        bridge,
        null,
      ),
    );
    final offset = ValueNotifier(0.0);
    final open = ValueNotifier(false);
    addTearDown(offset.dispose);
    addTearDown(open.dispose);
    final source = GlobalKey();
    const target = ValueKey('expanded-image');
    await tester.pumpWidget(
      MaterialApp(
        home: NativeProfileSheet(
          id: 1,
          expanded: false,
          offset: offset,
          child: ValueListenableBuilder<bool>(
            valueListenable: open,
            builder: (context, value, _) => ProfileSheetLayout(
              hero: SizedBox.square(key: source, dimension: 80),
              body: const SizedBox(height: 1000),
              portraitKey: source,
              photoOpen: value,
              enlargedPhoto: const ColoredBox(key: target, color: Colors.blue),
              onPhotoBack: () => open.value = false,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final closeCenter = tester.getCenter(find.byTooltip('Close sheet'));
    final panelWidth = tester.getSize(find.byType(ProfileSheetLayout)).width;
    open.value = true;
    await tester.pumpAndSettle();
    expect(tester.getBottomRight(find.byKey(target)).dy, greaterThan(340));
    final backCenter = tester.getCenter(find.byType(BackButton));
    expect(backCenter.dy, closeCenter.dy);
    expect(backCenter.dx, panelWidth - closeCenter.dx);
    await tester.tap(find.byType(BackButton));
    await tester.pumpAndSettle();
    expect(open.value, isFalse);
    expect(photoModes, [true, false]);
  });

  testWidgets('profile follows native bounce past both scroll limits', (
    tester,
  ) async {
    const channel = MethodChannel('buzz/profile_sheet');
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(channel, (_) async => null);
    addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
    final offset = ValueNotifier(0.0);
    addTearDown(offset.dispose);
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: NativeProfileSheet(
            id: 1,
            expanded: true,
            offset: offset,
            child: const ProfileSheetLayout(
              hero: Text('Portrait'),
              body: SizedBox(height: 1400),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final scroll = tester
        .widget<SingleChildScrollView>(find.byType(SingleChildScrollView))
        .controller!;
    final portraitTop = tester.getTopLeft(find.text('Portrait')).dy;
    offset.value = -30;
    await tester.pump();
    expect(tester.getTopLeft(find.text('Portrait')).dy, portraitTop + 30);
    final bottom = scroll.position.maxScrollExtent;
    offset.value = bottom + 24;
    await tester.pump();
    expect(scroll.offset, bottom + 24);
    offset.value = bottom;
    await tester.pumpAndSettle();
    expect(scroll.offset, bottom);
  });

  testWidgets(
    'sheet opens at the end of the header and expands before scrolling',
    (tester) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            userCacheProvider.overrideWith(
              () => _UserCache({
                scout: UserProfile(
                  pubkey: scout,
                  displayName: 'Scout',
                  nip05Handle: 'scout',
                  avatarUrl:
                      'data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20viewBox=%220%200%201%201%22/%3E',
                ),
              }),
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showUserProfileSheet(context, scout),
                  child: const Text('Open profile'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open profile'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Thinking'), findsNothing);
      expect(find.byType(NavigationBackdrop), findsOneWidget);
      final artwork = find.descendant(
        of: find.byKey(const ValueKey('profile-fixed-backdrop')),
        matching: find.byType(AvatarImageContent),
      );
      final portrait = find.descendant(
        of: find.byKey(const ValueKey('selected-profile-avatar')),
        matching: find.byType(AvatarImageContent),
      );
      expect(tester.getSize(artwork), tester.getSize(portrait));
      expect(
        tester.widget<AvatarImageContent>(artwork).imageUrl,
        tester.widget<AvatarImageContent>(portrait).imageUrl,
      );
      final cache = ProviderScope.containerOf(
        tester.element(find.byType(UserProfileSheet)),
      ).read(userStatusCacheProvider.notifier);
      final viewport = find.byType(SingleChildScrollView).first;
      final baseline = tester.getSize(viewport).height;
      cache.updateStatus(
        scout,
        const UserStatus(text: 'On a walk', emoji: '🌳', updatedAt: 1),
      );
      await tester.pumpAndSettle();
      expect(find.text('🌳 On a walk'), findsOneWidget);
      final oneLine = tester.getSize(viewport).height;
      expect(oneLine, greaterThan(baseline));
      cache.updateStatus(
        scout,
        const UserStatus(
          text: 'On a walk\nBack after lunch and available for questions later',
          emoji: '🌳',
          updatedAt: 2,
        ),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.getSize(viewport).height, closeTo(oneLine, .001));
      final ticker = tester
          .widgetList<SingleChildScrollView>(find.byType(SingleChildScrollView))
          .singleWhere(
            (view) =>
                view.scrollDirection == Axis.horizontal &&
                view.controller != null,
          );
      await tester.pump(const Duration(milliseconds: 100));
      final start = ticker.controller!.offset;
      await tester.pump(const Duration(seconds: 1));
      expect(ticker.controller!.offset - start, closeTo(24, .1));
      cache.updateStatus(scout, null);
      await tester.pumpAndSettle();
      expect(find.text('🌳 On a walk'), findsNothing);
      expect(tester.getSize(viewport).height, closeTo(baseline, .001));
      expect(
        tester.getTopLeft(find.text('Personal info')).dy,
        greaterThan(tester.getBottomLeft(viewport).dy),
      );
      final initialRect = tester.getRect(portrait);
      await tester.tap(portrait);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 120));
      final enlarged = find.byKey(const ValueKey('enlarged-profile-portrait'));
      double visibleWidth() {
        final box = tester.renderObject<RenderBox>(enlarged);
        return (box.localToGlobal(Offset(box.size.width, 0)) -
                box.localToGlobal(Offset.zero))
            .distance;
      }

      final halfway = visibleWidth();
      await tester.pumpAndSettle();
      expect(halfway, greaterThan(initialRect.width));
      expect(visibleWidth(), greaterThan(halfway));
      expect(
        find.descendant(of: enlarged, matching: find.byType(ClipOval)),
        findsWidgets,
      );
      await tester.drag(enlarged, const Offset(0, 120));
      await tester.pumpAndSettle();
      expect(enlarged, findsNothing);
      expect(tester.getSize(portrait), initialRect.size);
      final sheet = tester.widget<DraggableScrollableSheet>(
        find.byType(DraggableScrollableSheet),
      );
      expect(sheet.controller!.size, closeTo(1, .001));
      final scroll = tester.widget<SingleChildScrollView>(viewport).controller!;
      expect(scroll.position.maxScrollExtent, greaterThan(0));
      scroll.jumpTo(0);
      await tester.pump();
      final details = find.text('Personal info');
      final detailsTop = tester.getTopLeft(details).dy;
      await tester.drag(viewport, const Offset(0, -180));
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(details).dy, lessThan(detailsTop));
      expect(sheet.controller!.size, closeTo(1, .001));
    },
  );

  testWidgets('a supplied context follows profile changes while open', (
    tester,
  ) async {
    final cache = _UserCache({
      scout: UserProfile(pubkey: scout, displayName: 'Scout'),
      other: UserProfile(pubkey: other, displayName: 'Scout'),
    });
    final verified = _VerifiedProfiles();
    late IdentityNames opened;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(() => cache),
          verifiedProfilesProvider.overrideWith(() => verified),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Consumer(
            builder: (context, ref, _) {
              // The opener's context: both Scouts, so each has a suffix.
              opened = ref.read(identityNameSourcesProvider).scope([
                scout,
                other,
              ]);
              return Scaffold(
                body: UserProfileSheet(
                  pubkey: scout,
                  names: Provider.autoDispose<IdentityNames>(
                    (ref) => opened.withSources(
                      ref.watch(identityNameSourcesProvider),
                    ),
                  ),
                ),
              );
            },
          ),
        ),
      ),
    );
    await tester.pump();
    final suffixed = opened.labelFor(scout);
    expect(suffixed, startsWith('Scout · '));
    expect(find.text(suffixed), findsOneWidget);

    // The other Scout renames. The open sheet resolves the opener's context
    // against live facts, so its label no longer needs the suffix.
    cache.replace(UserProfile(pubkey: other, displayName: 'Renamed'));
    await tester.pump();
    expect(find.text('Scout'), findsOneWidget);
    expect(find.text(suffixed), findsNothing);

    // A community full name becomes the title, with the display name below.
    for (final checked in [true, false]) {
      verified.replace({
        scout: VerifiedProfile(
          displayName: 'Scout',
          username: 'scout',
          fullName: 'Scout Example',
          issuerLabel: 'Block',
          verifiedAt: 1000000,
          checked: checked,
          multipleIssuers: false,
        ),
      });
      await tester.pump();
      final title = find.text('Scout Example');
      final subtitle = find.text('Scout');
      expect(title, findsOneWidget);
      expect(subtitle, findsOneWidget);
      expect(
        tester.getTopLeft(title).dy,
        lessThan(tester.getTopLeft(subtitle).dy),
      );
      expect(
        tester.getCenter(title).dx,
        closeTo(tester.getCenter(subtitle).dx, 1),
      );
      expect(
        tester.widget<Text>(title).style!.fontSize,
        greaterThan(tester.widget<Text>(subtitle).style!.fontSize!),
      );
      expect(find.byKey(const ValueKey('verified-mark')), findsNothing);
      expect(find.textContaining('Name verified'), findsNothing);
      expect(find.textContaining('Last verified'), findsNothing);
      expect(find.textContaining('current account access'), findsNothing);
    }
    verified.replace({});
    await tester.pump();
    expect(find.text('Scout Example'), findsNothing);
    expect(find.text('Scout'), findsOneWidget);
  });
}
