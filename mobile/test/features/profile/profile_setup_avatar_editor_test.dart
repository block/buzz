import 'package:buzz/features/profile/profile_setup_avatar_editor.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/features/profile/profile_avatar_editor.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/profile_avatar_sheet'),
          (_) async => null,
        );
  });
  testWidgets(
    'accepted animated draft can reopen and finish without recapture',
    (tester) async {
      final draft = ProfileAnimatedAvatarDraft(
        animation: Uint8List(0),
        poster: Uint8List(0),
      );
      ProfileAvatarDraft? accepted;
      Rect? nativePreview;
      Future<void> open(ProfileAvatarDraft? initial) async {
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              home: ProfileSetupAvatarEditor(
                key: UniqueKey(),
                initialDraft: initial,
                onPreviewReady: (rect) async {
                  nativePreview = rect;
                },
                onFinished: (value) => accepted = value,
              ),
            ),
          ),
        );
        await tester.pump();
      }

      await open(null);
      tester
          .widget<ProfileAvatarEditor>(find.byType(ProfileAvatarEditor))
          .onModeChanged(ProfileAvatarMode.animated);
      await tester.pump();
      tester
          .widget<ProfileAvatarEditor>(find.byType(ProfileAvatarEditor))
          .onAnimatedPrepareChanged
          .call(() async => draft);
      await tester.pump();
      await tester.tap(find.text('Done'));
      await tester.pump();
      expect(accepted, same(draft));
      nativePreview = null;
      await open(accepted);
      expect(nativePreview, isNotNull);
      expect(nativePreview!.width, greaterThan(0));
      accepted = null;
      expect(find.byTooltip('Back to profile'), findsOneWidget);
      await tester.tap(find.text('Done'));
      await tester.pump();
      expect(accepted, same(draft));
      tester
          .widget<ProfileAvatarEditor>(find.byType(ProfileAvatarEditor))
          .onModeChanged(ProfileAvatarMode.image);
      await tester.pump();
      expect(
        tester
            .widget<TextButton>(find.widgetWithText(TextButton, 'Done'))
            .onPressed,
        isNull,
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('avatar draft editor lays out within a full height sheet', (
    tester,
  ) async {
    for (final anchor in <Rect?>[
      null,
      const Rect.fromLTWH(290, 200, 220, 220),
    ]) {
      Rect? previewRect;
      ProfileAvatarDraft? accepted;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: ProfileSetupAvatarEditor(
              key: ValueKey(anchor),
              anchoredPreview: anchor,
              onFinished: (draft) => accepted = draft,
              onPreviewReady: (rect) async {
                previewRect = rect;
              },
              initialDraft: ProfileUrlAvatarDraft(
                emojiAvatarDataUrl('🐝', 0xFFFFE75C),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(find.text('Done'), anchor == null ? findsOneWidget : findsNothing);
      expect(previewRect, isNotNull);
      expect(previewRect!.width, greaterThan(0));
      expect(previewRect!.height, greaterThan(0));
      if (anchor != null) {
        expect(previewRect, anchor);
        final circle = tester.getRect(
          find.byKey(const ValueKey('avatar-preview-position')),
        );
        final search = tester.getRect(
          find.byKey(const ValueKey('emoji-avatar-search')),
        );
        expect(circle.top - 40, closeTo(search.top - circle.bottom, 0.01));
        expect(find.text('Cancel'), findsNothing);
        expect(find.text('Profile photo'), findsNothing);
        // UIKit owns the buttons; exercise the same channel as its Done action.
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'buzz/profile_avatar_sheet',
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('reveal', {
              'x': 290.0,
              'y': 220.0,
              'width': 220.0,
              'height': 220.0,
            }),
          ),
          (_) {},
        );
        await tester.pump();
        final visibleAvatar = find.byKey(
          const ValueKey('avatar-editor-fixed-preview'),
        );
        expect(tester.getTopLeft(visibleAvatar).dy, 220);
        expect(tester.getSize(visibleAvatar), const Size.square(220));
        await tester.pump(const Duration(milliseconds: 100));
        expect(
          tester.getTopLeft(visibleAvatar).dy,
          allOf(greaterThan(66), lessThan(220)),
        );
        expect(tester.getSize(visibleAvatar), const Size.square(220));
        await tester.pump(const Duration(milliseconds: 100));
        expect(tester.getTopLeft(visibleAvatar).dy, closeTo(66, 0.01));
        // Empty native previews start 15% smaller, then reach editor size.
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'buzz/profile_avatar_sheet',
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('reveal', {
              'x': 306.5,
              'y': 236.5,
              'width': 187.0,
              'height': 187.0,
            }),
          ),
          (_) {},
        );
        await tester.pump();
        double paintedWidth() =>
            tester.getBottomRight(visibleAvatar).dx -
            tester.getTopLeft(visibleAvatar).dx;
        expect(paintedWidth(), closeTo(187, 0.01));
        expect(tester.getTopLeft(visibleAvatar).dy, closeTo(236.5, 0.01));
        await tester.pump(const Duration(milliseconds: 100));
        expect(paintedWidth(), allOf(greaterThan(187), lessThan(220)));
        await tester.pump(const Duration(milliseconds: 100));
        expect(paintedWidth(), closeTo(220, 0.01));
        // Saving returns to the full-size native avatar, without shrinking.
        var hideCompleted = false;
        final hiding = tester.binding.defaultBinaryMessenger
            .handlePlatformMessage(
              'buzz/profile_avatar_sheet',
              const StandardMethodCodec().encodeMethodCall(
                const MethodCall('hide', {
                  'x': 290.0,
                  'y': 220.0,
                  'width': 220.0,
                  'height': 220.0,
                }),
              ),
              (_) => hideCompleted = true,
            );
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 100));
        expect(paintedWidth(), closeTo(220, 0.01));
        expect(hideCompleted, isFalse);
        await tester.pump(const Duration(milliseconds: 100));
        await tester.pump(const Duration(milliseconds: 16));
        await tester.pump();
        expect(hideCompleted, isTrue);
        await hiding;
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'buzz/profile_avatar_sheet',
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('reveal'),
          ),
          (_) {},
        );
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 200));
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'buzz/profile_avatar_sheet',
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('done'),
          ),
          (_) {},
        );
        await tester.pump();
        expect(accepted, isA<ProfileUrlAvatarDraft>());
        await tester.tap(find.text('Image'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 200));
        expect(
          tester
              .getTopLeft(find.byKey(const ValueKey('avatar-preview-position')))
              .dy,
          220,
        );
        await tester.tap(find.text('Emoji'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 200));
        expect(
          tester
              .getTopLeft(find.byKey(const ValueKey('avatar-preview-position')))
              .dy,
          66,
        );
        expect(
          tester
              .getBottomLeft(find.byKey(const ValueKey('emoji-editor-emoji')))
              .dy,
          tester.getSize(find.byType(Scaffold)).height - 8,
        );
        await tester.tap(find.text('Animated'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 200));
        final capture = find.byKey(
          const ValueKey('animated-avatar-capture-preview'),
        );
        expect(tester.getSize(capture).width, 220);
        expect(tester.getTopLeft(capture).dy, 156);
      }
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });
}
