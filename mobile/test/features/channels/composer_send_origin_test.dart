import 'dart:async';

import 'package:buzz/features/channels/composer_send_origin.dart';
import 'package:buzz/features/channels/local_message_send_animation_provider.dart';
import 'package:buzz/features/channels/local_message_send_transition.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('slow sends bind submitted geometry to their own local claim', (
    tester,
  ) async {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final notifier = container.read(
      localMessageSendAnimationProvider('channel').notifier,
    );
    final key = GlobalKey();
    expect(ComposerSendOrigin.capture(key), isNull);
    Future<void> layout(double height) => tester.pumpWidget(
      MaterialApp(
        home: Align(
          alignment: Alignment.bottomCenter,
          child: SizedBox(key: key, width: 320, height: height),
        ),
      ),
    );
    await layout(120);
    final submitted = ComposerSendOrigin.capture(key);
    expect(submitted, tester.getRect(find.byKey(key)));
    final waitForUpload = Completer<void>();
    final upload = ComposerSendOrigin.run(submitted, () async {
      await waitForUpload.future;
      notifier.mark('upload');
    });
    await layout(44);
    final secondOrigin = ComposerSendOrigin.capture(key);
    expect(secondOrigin, tester.getRect(find.byKey(key)));
    expect(secondOrigin, isNot(submitted));
    final invite = ComposerSendOrigin.run(secondOrigin, () async {
      await Future<void>.delayed(const Duration(seconds: 3));
      notifier.mark('invite');
    });
    await tester.pump(const Duration(seconds: 3));
    await invite;
    waitForUpload.complete();
    await upload;
    expect(notifier.originFor('upload'), submitted);
    expect(notifier.originFor('invite'), secondOrigin);
    expect(ComposerSendOrigin.current, isNull);
    notifier.mark('unrelated');
    expect(notifier.originFor('unrelated'), isNull);
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(
          home: LocalMessageSendTransition(
            animate: true,
            channelId: 'channel',
            messageId: 'upload',
            child: SizedBox(width: 100, height: 80),
          ),
        ),
      ),
    );
    expect(
      tester.widget<LocalSendProgress>(find.byType(LocalSendProgress)).origin,
      submitted,
    );
    await tester.pumpAndSettle();
  });
}
