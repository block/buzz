import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_sections/pin_shortcuts.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/widgets/dm_header_cluster.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:buzz/shared/widgets/avatar_shape.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets(
    'pins appear only when there are conversation destinations',
    (tester) async {
      tester.view.physicalSize = const Size(420, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      Channel channel(int index) => Channel(
        id: 'channel-$index',
        name: 'General $index',
        description: '',
        createdBy: 'owner',
        createdAt: DateTime(2026),
        memberCount: 1,
        visibility: 'open',
        channelType: 'stream',
        isMember: true,
      );
      String? held;
      var pins = <Channel>[];
      late StateSetter updatePins;
      final pane = ConversationPaneController();
      addTearDown(pane.dispose);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pinMemberSnapshotProvider.overrideWith(
              (ref, id) => [
                for (final key in ['alice', 'bob'])
                  ChannelMember(
                    pubkey: key,
                    role: 'member',
                    joinedAt: DateTime(2026),
                  ),
              ],
            ),
          ],
          child: ConversationPaneScope(
            controller: pane,
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Padding(
                  padding: const EdgeInsets.all(16),
                  child: StatefulBuilder(
                    builder: (context, setState) {
                      updatePins = setState;
                      return HomePins(
                        channels: pins,
                        viewer: 'owner',
                        label: (c) => c.name,
                        onLongPress: (channel) => held = channel.id,
                        onOpen: (c) => pane.open(c.id, (_) => const SizedBox()),
                      );
                    },
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      expect(tester.getSize(find.byType(HomePins)).height, 0);
      final phone = defaultTargetPlatform == TargetPlatform.iOS;
      final columns = phone ? 3 : 4;
      updatePins(() => pins = List.generate(5, channel));
      await tester.pump();
      expect(find.bySemanticsLabel(RegExp('Add pin, slot')), findsNothing);
      await tester.pump(const Duration(milliseconds: 90));
      final entering = find
          .descendant(
            of: find.byKey(const ValueKey('channel-0')),
            matching: find.byType(Transform),
          )
          .first;
      expect(tester.widget<Transform>(entering).transform.entry(0, 0), 0);
      await tester.pump(const Duration(milliseconds: 190));
      await tester.pump(const Duration(milliseconds: 90));
      expect(
        tester.widget<Transform>(entering).transform.entry(0, 0),
        inExclusiveRange(0, 1),
      );
      await tester.pumpAndSettle();
      final filled = find.bySemanticsLabel(RegExp('Open pinned General'));
      expect(
        tester.getTopLeft(filled.at(columns - 1)).dy,
        tester.getTopLeft(filled.first).dy,
      );
      expect(tester.getTopRight(filled.at(columns - 1)).dx, 420 - 16 - Grid.xs);
      expect(
        tester.getTopLeft(filled.at(columns)).dx,
        tester.getTopLeft(filled.first).dx,
      );
      expect(
        tester.getTopLeft(filled.at(columns)).dy,
        greaterThan(tester.getTopLeft(filled.first).dy),
      );
      final ring = find.byWidgetPredicate(
        (widget) => widget is ClipPath && widget.clipper is AvatarRingClipper,
      );
      expect(ring, findsNothing);
      final caption = find.text('General 0');
      expect(caption, findsOneWidget);
      expect(tester.getCenter(caption).dx, tester.getCenter(filled.first).dx);
      await tester.longPress(caption);
      expect(held, 'channel-0');
      expect(pane.selectedId, isNull);
      await tester.tap(filled.first);
      await tester.pump();
      expect(ring, findsOneWidget);
      await tester.pump();
      expect(
        tester
            .widgetList<DmHeaderCluster>(find.byType(DmHeaderCluster))
            .map((cluster) => cluster.count),
        everyElement(2),
      );
      expect(find.byType(DmHeaderCluster), findsNWidgets(5));
      expect(pane.selectedId, 'channel-0');
      final formerFirst = tester.getTopLeft(filled.first);
      final nextPin = find.bySemanticsLabel('Open pinned General 1');
      final originalNext = tester.getTopLeft(nextPin);
      updatePins(() => pins = pins.sublist(1));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 90));
      expect(
        tester.widget<Transform>(entering).transform.entry(0, 0),
        inExclusiveRange(0, 1),
      );
      expect(tester.getTopLeft(nextPin), originalNext);
      await tester.pump(const Duration(milliseconds: 190));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('channel-0')), findsNothing);
      expect(
        tester.getTopLeft(find.bySemanticsLabel('Open pinned General 1')),
        formerFirst,
      );
      updatePins(() => pins = pins.take(2).toList());
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 280));
      await tester.pumpAndSettle();
      expect(tester.getCenter(filled.first).dx, 121);
      expect(tester.getCenter(filled.last).dx, 299);
      updatePins(() => pins = pins.take(1).toList());
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 280));
      await tester.pumpAndSettle();
      expect(tester.getCenter(filled.first).dx, 210);
      updatePins(() => pins = []);
      await tester.pumpAndSettle();
      expect(tester.getSize(find.byType(HomePins)).height, 0);
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant({
      TargetPlatform.android,
      TargetPlatform.iOS,
    }),
  );
}
