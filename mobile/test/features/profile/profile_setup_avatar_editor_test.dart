import 'package:buzz/features/profile/profile_setup_avatar_editor.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'dart:typed_data';
import 'package:buzz/features/profile/profile_avatar_editor.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
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
    Rect? previewRect;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          home: ProfileSetupAvatarEditor(
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
    expect(find.text('Done'), findsOneWidget);
    expect(previewRect, isNotNull);
    expect(previewRect!.width, greaterThan(0));
    expect(previewRect!.height, greaterThan(0));
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
  });
}
