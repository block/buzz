import 'package:buzz/features/channels/reply_summary_link.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  for (final mirrored in [false, true]) {
    for (final reducedMotion in [false, true]) {
      testWidgets(
        'thread footer keeps pending work and replies at the thread position: mirrored=$mirrored reduced=$reducedMotion',
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

          complete.value = true;
          await tester.pump();
          double endPosition() => mirrored
              ? tester.getRect(find.text('1 reply')).right
              : tester.getRect(find.byType(SmallAvatar)).left;
          final start = endPosition();
          await tester.pump(const Duration(milliseconds: 100));
          final middle = endPosition();
          await tester.pump(const Duration(milliseconds: 150));
          final end = endPosition();
          expect(
            end,
            closeTo(mirrored ? lane.right - 32 : lane.left + 32, 0.1),
          );
          expect(start, closeTo(end, 0.1));
          expect(middle, closeTo(end, 0.1));
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
}

class _Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'agent': UserProfile(pubkey: 'agent', displayName: 'Agent'),
  };
}
