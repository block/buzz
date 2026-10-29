import 'dart:async';
import 'package:buzz/features/agents/agent_avatar_picker.dart';
import 'package:buzz/shared/agents/agent_avatar_catalog.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets(
    'scrubbing changes the selection and emoji mode exports an avatar',
    (tester) async {
      late Completer<void> rendered;
      var selection = AgentAvatarSelection.url(agentAvatarCatalog.first.url);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            emojiDatasetProvider.overrideWith(
              (ref) async => EmojiDataset.empty,
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
      await tester.drag(find.byType(PageView), const Offset(-190, 0));
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
      expect(find.byType(GridView), findsOneWidget);
      await tester.tap(find.text('Gloopies'));
      await tester.pumpAndSettle();
      expect(selection.url, character);
      expect(tester.takeException(), isNull);
    },
  );
}
