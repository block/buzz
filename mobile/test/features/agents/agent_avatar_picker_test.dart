import 'dart:async';
import 'package:camera_platform_interface/camera_platform_interface.dart';
import 'package:buzz/features/agents/agent_avatar_picker.dart';
import 'package:buzz/shared/agents/agent_avatar_catalog.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('grid selection and shared emoji controls preserve the avatar', (
    tester,
  ) async {
    final previousCamera = CameraPlatform.instance;
    CameraPlatform.instance = _NoCameras();
    addTearDown(() => CameraPlatform.instance = previousCamera);
    late Completer<void> rendered;
    var selection = AgentAvatarSelection.url(agentAvatarCatalog.first.url);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
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
    await tester.tap(find.byKey(const ValueKey('agent-avatar-1')));
    await tester.pumpAndSettle();
    expect(selection.url, isNot(agentAvatarCatalog.first.url));
    final character = selection.url;
    await tester.runAsync(() async {
      rendered = Completer<void>();
      tester
          .widget<SegmentedButton<int>>(find.byType(SegmentedButton<int>))
          .onSelectionChanged!({1});
      await rendered.future.timeout(const Duration(seconds: 5));
    });

    await tester.pumpAndSettle();
    expect(selection.bytes, isNotNull);
    expect(
      (tester
                  .widget<GridView>(
                    find.byKey(const ValueKey('emoji-avatar-grid')),
                  )
                  .gridDelegate
              as SliverGridDelegateWithFixedCrossAxisCount)
          .crossAxisCount,
      6,
    );
    expect(find.byKey(const ValueKey('emoji-avatar-search')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('emoji-avatar-skin-tone')),
      findsOneWidget,
    );
    await tester.ensureVisible(
      find.byKey(const ValueKey('emoji-editor-background')),
    );
    await tester.tap(find.byKey(const ValueKey('emoji-editor-background')));
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('emoji-background-editor')),
      findsOneWidget,
    );
    await tester.ensureVisible(find.text('Gloopies'));
    await tester.tap(find.text('Gloopies'));
    await tester.pumpAndSettle();
    expect(selection.url, character);
    await tester.tap(find.text('Image'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('image-source-camera')), findsOneWidget);
    expect(find.byKey(const ValueKey('image-source-library')), findsOneWidget);
    expect(selection.url, character);

    await tester.ensureVisible(
      find.byKey(const ValueKey('image-source-camera')),
    );
    await tester.tap(find.byKey(const ValueKey('image-source-camera')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('image-avatar-camera')), findsOneWidget);
    await tester.ensureVisible(
      find.byKey(const ValueKey('image-camera-left-action')),
    );
    await tester.tap(find.byKey(const ValueKey('image-camera-left-action')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('image-source-library')), findsOneWidget);
    expect(selection.url, character);
    expect(tester.takeException(), isNull);
  });
}

class _NoCameras extends CameraPlatform {
  @override
  Future<List<CameraDescription>> availableCameras() async => [];
}
