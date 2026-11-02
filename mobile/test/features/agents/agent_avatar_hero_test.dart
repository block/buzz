import 'dart:convert';

import 'package:buzz/features/agents/agent_avatar_hero.dart';
import 'package:buzz/features/agents/agent_avatar_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('return flight shows the retained selection on Cancel and Save', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final original = base64Decode(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
    );
    final draft = Uint8List.fromList(original);
    for (final save in [false, true]) {
      final selection = ValueNotifier(AgentAvatarSelection.image(original));
      await tester.pumpWidget(
        MaterialApp(
          home: ValueListenableBuilder<AgentAvatarSelection>(
            valueListenable: selection,
            builder: (context, value, _) => Scaffold(
              body: Column(
                children: [
                  AgentAvatarHero(
                    tag: 'avatar',
                    selection: value,
                    child: const SizedBox.square(dimension: 100),
                  ),
                  TextButton(
                    onPressed: () async {
                      final result = await Navigator.of(context)
                          .push<AgentAvatarSelection>(
                            PageRouteBuilder(
                              transitionDuration: const Duration(
                                milliseconds: 300,
                              ),
                              reverseTransitionDuration: const Duration(
                                milliseconds: 300,
                              ),
                              pageBuilder: (context, _, _) => Scaffold(
                                body: Column(
                                  children: [
                                    AgentAvatarHero(
                                      tag: 'avatar',
                                      selection: AgentAvatarSelection.image(
                                        draft,
                                      ),
                                      child: const SizedBox.square(
                                        dimension: 200,
                                      ),
                                    ),
                                    TextButton(
                                      onPressed: () =>
                                          Navigator.of(context).pop(
                                            save
                                                ? AgentAvatarSelection.image(
                                                    draft,
                                                  )
                                                : null,
                                          ),
                                      child: const Text('Return'),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          );
                      if (result != null) selection.value = result;
                    },
                    child: const Text('Edit'),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Edit'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Return'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      final image = tester.widget<Image>(find.byType(Image));
      expect((image.image as MemoryImage).bytes, same(save ? draft : original));
      await tester.pumpAndSettle();
      expect(selection.value.bytes, same(save ? draft : original));
      await tester.pumpWidget(const SizedBox.shrink());
      selection.dispose();
    }
    debugDefaultTargetPlatformOverride = null;
  });
}
