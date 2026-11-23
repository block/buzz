import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/widgets/duo_toolbar.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('photo gallery uses the Duo rail and full image surface', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(950, 670);
    addTearDown(tester.view.reset);
    final navigator = GlobalKey<NavigatorState>();
    final side = ValueNotifier<bool>(false);
    addTearDown(side.dispose);
    var replies = 0;
    String? selected;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          navigatorKey: navigator,
          builder: (context, child) => ValueListenableBuilder<bool>(
            valueListenable: side,
            builder: (context, left, _) => DuoToolbarScope(
              active: true,
              strip: FoldStrip(
                side: left ? FoldStripSide.left : FoldStripSide.right,
                rect: Rect.fromLTWH(left ? 0 : 870, 0, 80, 670),
                topClearance: 110,
                bottomClearance: 0,
                isClearanceEstimated: false,
              ),
              child: child!,
            ),
          ),
          home: const Scaffold(body: Text('Conversation')),
        ),
      ),
    );
    void open() {
      navigator.currentState!.push(
        buildImageViewerRoute(
          imageUrl: 'https://example.org/one.png',
          heroTag: 'one',
          galleryItems: const [
            MediaViewerImage(
              url: 'https://example.org/one.png',
              heroTag: 'one',
            ),
            MediaViewerImage(
              url: 'https://example.org/two.png',
              heroTag: 'two',
            ),
          ],
          onReply: () => replies++,
          onMore: (_, url) => selected = url,
        ),
      );
    }

    Finder rail(String label) => find.byKey(ValueKey('duo-toolbar-$label'));
    void invoke(String label) {
      tester
          .widget<IosNavigationBar>(
            find.descendant(
              of: rail(label),
              matching: find.byType(IosNavigationBar),
            ),
          )
          .leading!
          .onPressed!();
    }

    open();
    await tester.pumpAndSettle();
    for (final left in [false, true]) {
      side.value = left;
      await tester.pumpAndSettle();
      double y = 0;
      for (final label in [
        'Close image viewer',
        'Reply in thread',
        'More image actions',
      ]) {
        final rect = tester.getRect(rail(label));
        expect(rect.center.dx, left ? lessThan(80) : greaterThan(870));
        expect(rect.top, greaterThan(y));
        y = rect.top;
      }
      expect(
        tester.getRect(
          find.byKey(const ValueKey('message-media-image-viewer-gesture:0')),
        ),
        const Rect.fromLTWH(0, 0, 950, 670),
      );
      expect(
        find.byKey(const ValueKey('message-media-image-viewer-reply-thread')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('message-media-image-viewer-more-actions')),
        findsNothing,
      );
    }
    await tester.drag(
      find.byKey(const ValueKey('message-media-image-viewer-pages')),
      const Offset(-800, 0),
    );
    await tester.pumpAndSettle();
    invoke('More image actions');
    expect(selected, 'https://example.org/two.png');
    invoke('Reply in thread');
    await tester.pumpAndSettle();
    expect(replies, 1);
    expect(rail('Close image viewer'), findsNothing);
    open();
    await tester.pumpAndSettle();
    invoke('Close image viewer');
    await tester.pumpAndSettle();
    expect(find.text('Conversation'), findsOneWidget);
    expect(replies, 1);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });
}
