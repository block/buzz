import 'package:buzz/features/channels/reply_summary_link.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  for (final mirrored in [false, true]) {
    for (final reducedMotion in [false, true]) {
      testWidgets(
        'thread footer morphs working content while keeping the thread elbow fixed: mirrored=$mirrored reduced=$reducedMotion',
        (tester) async {
          final complete = ValueNotifier(false);
          addTearDown(complete.dispose);
          var opened = 0;
          await tester.pumpWidget(
            WidgetHelpers.testable(
              disableAnimations: reducedMotion,
              overrides: [userCacheProvider.overrideWith(_Profiles.new)],
              child: SizedBox(
                key: const ValueKey('lane'),
                width: 400,
                child: Align(
                  alignment: mirrored ? Alignment.topRight : Alignment.topLeft,
                  child: ValueListenableBuilder(
                    valueListenable: complete,
                    builder: (context, done, _) => ReplySummaryLink(
                      mirrored: mirrored,
                      summary: done
                          ? const ThreadSummary(
                              threadHeadId: 'root',
                              replyCount: 1,
                              participantPubkeys: ['agent'],
                            )
                          : null,
                      working: done ? const [] : const ['agent'],
                      workingNames: done ? const [] : const ['Agent'],
                      onTap: () => opened++,
                    ),
                  ),
                ),
              ),
            ),
          );
          await tester.pump();
          final lane = tester.getRect(find.byKey(const ValueKey('lane')));
          expect(
            tester.getCenter(find.byType(SmallAvatar)).dy,
            closeTo(tester.getCenter(find.byType(ReplySummaryLink)).dy, 0.1),
          );
          final pending = mirrored
              ? tester
                    .getRect(
                      find.byKey(const ValueKey('thread-working-container')),
                    )
                    .right
              : tester
                    .getRect(
                      find.byKey(const ValueKey('thread-working-container')),
                    )
                    .left;
          expect(
            pending,
            closeTo(mirrored ? lane.right - 32 : lane.left + 32, 0.1),
          );
          expect(
            find.bySemanticsLabel('View thread: Agent is working'),
            findsOneWidget,
          );
          await tester.tap(find.byType(ReplySummaryLink));
          expect(opened, 1);

          final avatarElement = tester.element(find.byType(SmallAvatar));
          final avatarBefore = tester.getTopLeft(find.byType(SmallAvatar)).dx;
          complete.value = true;
          await tester.pump();
          double endPosition() => mirrored
              ? tester.getRect(find.text('1 reply')).right
              : tester.getRect(find.byType(SmallAvatar)).left;
          final start = endPosition();
          expect(tester.element(find.byType(SmallAvatar)), same(avatarElement));
          if (!reducedMotion) {
            expect(
              tester.getTopLeft(find.byType(SmallAvatar)).dx,
              closeTo(avatarBefore, 0.1),
            );
          }
          await tester.pump(const Duration(milliseconds: 100));
          final middle = tester.getTopLeft(find.byType(SmallAvatar)).dx;
          final background =
              tester
                      .widget<DecoratedBox>(
                        find.byKey(const ValueKey('thread-working-container')),
                      )
                      .decoration
                  as BoxDecoration;
          expect(background.color!.a, reducedMotion ? 0 : closeTo(0.5, 0.1));
          await tester.pump(const Duration(milliseconds: 150));
          final end = endPosition();
          final paragraph = tester.renderObject<RenderParagraph>(
            find.text('1 reply'),
          );
          final glyphBox = paragraph
              .getBoxesForSelection(
                const TextSelection(baseOffset: 0, extentOffset: 7),
              )
              .first
              .toRect();
          expect(
            paragraph.localToGlobal(glyphBox.center).dy,
            closeTo(tester.getCenter(find.byType(SmallAvatar)).dy, 1),
          );
          expect(
            end,
            closeTo(mirrored ? lane.right - 32 : lane.left + 32, 0.1),
          );
          if (reducedMotion) {
            expect(start, closeTo(end, 0.1));
          } else {
            expect(
              (tester.getTopLeft(find.byType(SmallAvatar)).dx - middle).abs(),
              greaterThan(0.1),
            );
          }
          expect(tester.element(find.byType(SmallAvatar)), same(avatarElement));
          expect(
            find.byKey(const ValueKey('thread-working-dots')),
            findsNothing,
          );
          expect(find.bySemanticsLabel('View thread: 1 reply'), findsOneWidget);
          await tester.tap(find.byType(ReplySummaryLink));
          expect(opened, 2);
          expect(tester.takeException(), isNull);
        },
      );
    }
  }
  testWidgets(
    'reply arriving before typing ends keeps its count and can resume work',
    (tester) async {
      Widget view({required bool working, required bool replied}) =>
          WidgetHelpers.testable(
            overrides: [userCacheProvider.overrideWith(_Profiles.new)],
            child: Align(
              alignment: Alignment.topRight,
              child: ReplySummaryLink(
                mirrored: true,
                summary: replied
                    ? const ThreadSummary(
                        threadHeadId: 'root',
                        replyCount: 1,
                        participantPubkeys: ['agent'],
                      )
                    : null,
                working: working ? const ['agent'] : const [],
                onTap: () {},
              ),
            ),
          );
      await tester.pumpWidget(view(working: true, replied: false));
      final avatar = tester.element(find.byType(SmallAvatar));
      await tester.pumpWidget(view(working: true, replied: true));
      expect(find.text('1 reply'), findsOneWidget);
      expect(find.byKey(const ValueKey('thread-working-dots')), findsOneWidget);
      await tester.pumpWidget(view(working: false, replied: true));
      await tester.pump(const Duration(milliseconds: 100));
      await tester.pumpWidget(view(working: true, replied: true));
      await tester.pump(const Duration(milliseconds: 250));
      expect(tester.element(find.byType(SmallAvatar)), same(avatar));
      expect(find.text('1 reply'), findsOneWidget);
      expect(find.byKey(const ValueKey('thread-working-dots')), findsOneWidget);
      await tester.pumpWidget(view(working: false, replied: true));
      await tester.pump(const Duration(milliseconds: 250));
      expect(find.byKey(const ValueKey('thread-working-dots')), findsNothing);
      expect(tester.takeException(), isNull);
    },
  );
}

class _Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'agent': UserProfile(pubkey: 'agent', displayName: 'Agent'),
  };
}
