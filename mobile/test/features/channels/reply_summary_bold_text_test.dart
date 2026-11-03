import 'package:buzz/features/channels/reply_summary_link.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  setUpAll(() async {
    // Ahem has identical regular/bold metrics and hides this regression.
    final font = FontLoader('Inter')
      ..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'));
    await font.load();
  });

  for (final working in [false, true]) {
    for (final scale in [1.0, 2.0]) {
      testWidgets(
        'reply label honors Bold Text: working=$working scale=$scale',
        (tester) async {
          Widget view(bool bold) => WidgetHelpers.testable(
            overrides: [userCacheProvider.overrideWith(_Profiles.new)],
            child: MediaQuery(
              data: MediaQueryData(
                boldText: bold,
                textScaler: TextScaler.linear(scale),
              ),
              child: Align(
                alignment: Alignment.topLeft,
                child: SizedBox(
                  width: 400,
                  child: ReplySummaryLink(
                    summary: const ThreadSummary(
                      threadHeadId: 'root',
                      replyCount: 12,
                      participantPubkeys: ['participant'],
                    ),
                    working: working ? const ['participant'] : const [],
                    onTap: () {},
                  ),
                ),
              ),
            ),
          );

          for (final bold in [false, true, false]) {
            await tester.pumpWidget(view(bold));
            final paragraph = tester.renderObject<RenderParagraph>(
              find.text('12 replies'),
            );
            final expected = TextPainter(
              text: paragraph.text,
              textDirection: paragraph.textDirection,
              textScaler: paragraph.textScaler,
            )..layout();
            // The entire one-line label fits at this viewport; its allocated
            // width must match the text actually rendered, including bold.
            expect(paragraph.size.width, closeTo(expected.width, 0.01));
            expect(paragraph.size.height, closeTo(expected.height, 0.01));
            expect(paragraph.didExceedMaxLines, isFalse);
            if (bold) {
              expect(paragraph.text.style!.fontWeight, FontWeight.bold);
            }
            expected.dispose();
            expect(tester.takeException(), isNull);
          }
        },
      );
    }
  }
}

class _Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'participant': UserProfile(
      pubkey: 'participant',
      displayName: 'Participant',
    ),
  };
}
