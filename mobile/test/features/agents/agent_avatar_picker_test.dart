import 'package:buzz/features/agents/agent_avatar_category.dart';
import 'dart:async';
import 'dart:convert';
import 'package:buzz/features/agents/agent_avatar_choices.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/features/agents/agent_avatar_picker.dart';
import 'package:buzz/shared/agents/agent_avatar_catalog.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/widgets/avatar_shape.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('reopening restores the avatar mode and reveals its category', (
    tester,
  ) async {
    final png = base64Decode(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
    );
    final cases = [
      (AgentAvatarSelection.image(png), AgentAvatarCategory.image, 'Image'),
      (
        AgentAvatarSelection.emoji(png, '😊', 0xFFFFCC00),
        AgentAvatarCategory.emoji,
        'Emoji',
      ),
    ];
    for (final (selection, mode, label) in cases) {
      for (final scale in [1.0, 1.3, 1.5, 2.0]) {
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              emojiDatasetProvider.overrideWith(
                (ref) async => const EmojiDataset(
                  categories: [],
                  all: [],
                  nativeToShortcode: {},
                ),
              ),
            ],
            child: MaterialApp(
              builder: (context, child) => MediaQuery(
                data: MediaQuery.of(context).copyWith(
                  disableAnimations: true,
                  textScaler: TextScaler.linear(scale),
                ),
                child: child!,
              ),
              home: Scaffold(
                body: SingleChildScrollView(
                  child: AgentAvatarPicker(
                    selection: selection,
                    onChanged: (_) {},
                    disabled: false,
                    onBusyChanged: (_) {},
                  ),
                ),
              ),
            ),
          ),
        );
        // The short test viewport scrolls vertically to the bottom category row.
        await tester.pump();
        await tester.ensureVisible(find.byType(AgentAvatarChoices));
        await tester.pump();
        expect(
          tester
              .widget<AgentAvatarChoices>(find.byType(AgentAvatarChoices))
              .category,
          mode,
        );
        expect(
          find.byKey(ValueKey('agent-avatar-choice-$label')).hitTestable(),
          findsOneWidget,
        );
        expect(tester.takeException(), isNull);
      }
    }
  });

  testWidgets(
    'filmstrip selection and shared emoji controls preserve the avatar',
    (tester) async {
      var galleryRequests = 0;
      late Completer<void> rendered;
      var selection = AgentAvatarSelection.url(agentAvatarCatalog.first.url);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            mediaUploadServiceProvider.overrideWithValue(
              MediaUploadService(
                baseUrl: 'https://relay.example',
                nsec: null,
                pickGalleryVideo: () async => null,
                pickGalleryImage: () async {
                  galleryRequests++;
                  return null;
                },
              ),
            ),
            emojiDatasetProvider.overrideWith(
              (ref) async => const EmojiDataset(
                categories: [],
                all: [
                  EmojiEntry(
                    id: 'smile',
                    name: 'Smile',
                    keywords: [],
                    native: '😊',
                    categoryId: 'people',
                  ),
                ],
                nativeToShortcode: {},
              ),
            ),
          ],
          child: MaterialApp(
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(disableAnimations: true),
              child: child!,
            ),
            home: Scaffold(
              body: StatefulBuilder(
                builder: (context, setState) => SingleChildScrollView(
                  child: AgentAvatarPicker(
                    selection: selection,
                    disabled: false,
                    onBusyChanged: (_) {},
                    onChanged: (value) {
                      setState(() => selection = value);
                      if (value.bytes != null && !rendered.isCompleted) {
                        rendered.complete();
                      }
                    },
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.byKey(const ValueKey('agent-avatar-1')));
      await tester.tap(find.byKey(const ValueKey('agent-avatar-1')));
      await tester.pumpAndSettle();
      expect(selection.url, isNot(agentAvatarCatalog.first.url));
      final character = selection.url;
      final semantics = tester.ensureSemantics();
      expect(
        find.bySemanticsLabel(
          'Selected avatar: ${agentAvatarCatalog.firstWhere((item) => item.url == character).name}',
        ),
        findsOneWidget,
      );
      semantics.dispose();
      final examples = Map.of(
        tester
            .widget<AgentAvatarChoices>(find.byType(AgentAvatarChoices))
            .examples,
      );
      for (final pack in AgentAvatarPack.values.skip(1)) {
        await tester.ensureVisible(find.text(pack.label));
        await tester.tap(find.text(pack.label));
        await tester.pumpAndSettle();
        expect(
          pack.indices.map((i) => agentAvatarCatalog[i].url),
          contains(selection.url),
        );
        final grid = tester.widget<ListView>(
          find.byKey(const ValueKey('agent-avatar-filmstrip')),
        );
        expect(
          grid.childrenDelegate.estimatedChildCount,
          pack.indices.length - 1,
        );
      }
      final rememberedPack = AgentAvatarPack.values.last;
      final alternate = rememberedPack.indices.firstWhere(
        (i) => agentAvatarCatalog[i].url != selection.url,
      );
      await tester.ensureVisible(
        find.byKey(ValueKey('agent-avatar-$alternate')),
      );
      await tester.tap(find.byKey(ValueKey('agent-avatar-$alternate')));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Buzzies'));
      await tester.tap(find.text('Buzzies'));
      await tester.pumpAndSettle();
      expect(selection.url, character);
      await tester.ensureVisible(find.text(rememberedPack.label));
      await tester.tap(find.text(rememberedPack.label));
      await tester.pumpAndSettle();
      expect(selection.url, agentAvatarCatalog[alternate].url);
      await tester.ensureVisible(find.text('Buzzies'));
      await tester.tap(find.text('Buzzies'));
      await tester.pumpAndSettle();
      expect(selection.url, character);

      expect(
        tester
            .widget<AgentAvatarChoices>(find.byType(AgentAvatarChoices))
            .examples,
        examples,
      );

      // Category navigation stays on the top buttons, not preview drags.
      final beforeDrag = selection.url;
      await tester.drag(find.byType(AgentAvatarPicker), const Offset(-300, 0));
      await tester.pumpAndSettle();
      expect(selection.url, beforeDrag);
      expect(find.byType(PageView), findsNothing);

      // Every preview uses the same squircle the agent gets once created.
      void expectSquirclePreview() {
        final previewClip = tester.widget<ClipPath>(
          find.descendant(
            of: find.byWidgetPredicate(
              (widget) =>
                  widget is Semantics &&
                  (widget.properties.label?.startsWith('Selected avatar:') ==
                          true ||
                      widget.properties.label == 'Selected agent avatar'),
            ),
            matching: find.byType(ClipPath),
          ),
        );
        expect(
          (previewClip.clipper! as ShapeBorderClipper).shape,
          avatarShape(isAgent: true),
        );
      }

      expectSquirclePreview();
      await tester.runAsync(() async {
        rendered = Completer<void>();
        await tester.ensureVisible(
          find.byKey(const ValueKey('agent-avatar-choice-Emoji')),
        );
        await tester.tap(
          find.byKey(const ValueKey('agent-avatar-choice-Emoji')),
        );
        await rendered.future.timeout(const Duration(seconds: 5));
      });

      await tester.pumpAndSettle();
      expect(selection.bytes, isNotNull);
      expectSquirclePreview();
      expect(find.byKey(const ValueKey('agent-emoji-colors')), findsOneWidget);
      expect(find.byKey(const ValueKey('agent-choose-emoji')), findsOneWidget);
      expect(find.byKey(const ValueKey('emoji-avatar-grid')), findsNothing);
      await tester.runAsync(() async {
        rendered = Completer<void>();
        await tester.ensureVisible(
          find.byKey(const ValueKey('agent-emoji-color-1')),
        );
        await tester.tap(find.byKey(const ValueKey('agent-emoji-color-1')));
        await rendered.future.timeout(const Duration(seconds: 5));
      });
      await tester.pumpAndSettle();
      expect(selection.color, 0xFFFFE75C);
      await tester.ensureVisible(find.text('Buzzies'));
      await tester.tap(find.text('Buzzies'));
      await tester.pumpAndSettle();
      expect(selection.url, character);
      await tester.ensureVisible(find.text('Image'));
      await tester.tap(find.text('Image'));
      await tester.pumpAndSettle();
      expect(galleryRequests, 1);
      expect(find.text('Camera'), findsNothing);
      expect(find.byKey(const ValueKey('image-source-library')), findsNothing);
      expect(selection.url, character);
      final restoredChoices = tester.widget<AgentAvatarChoices>(
        find.byType(AgentAvatarChoices),
      );

      expect(restoredChoices.category.pack, AgentAvatarPack.buzzies);
      expect(find.byKey(const ValueKey('image-avatar-camera')), findsNothing);
      expect(find.text('Animation'), findsNothing);
      expect(tester.takeException(), isNull);
    },
  );
}
