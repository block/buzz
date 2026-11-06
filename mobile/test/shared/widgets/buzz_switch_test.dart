import 'package:buzz/shared/widgets/buzz_switch.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'native switch sends changes and follows restored or disabled state',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final calls = <MethodCall>[];
      const channel = MethodChannel('buzz/switch/4242');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        calls.add(call);
        return null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      final changes = <bool>[];
      Widget app(bool value, {bool enabled = true}) => MaterialApp(
        home: BuzzSwitch(
          value: value,
          label: 'Voice notes',
          onChanged: enabled ? changes.add : null,
        ),
      );
      await tester.pumpWidget(app(true));
      final view = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(view.viewType, 'buzz/switch');
      expect(view.creationParams, containsPair('label', 'Voice notes'));
      view.onPlatformViewCreated!(4242);
      await tester.pump();
      Future<void> change(bool value) =>
          tester.binding.defaultBinaryMessenger.handlePlatformMessage(
            channel.name,
            const StandardMethodCodec().encodeMethodCall(
              MethodCall('changed', value),
            ),
            (_) {},
          );
      await change(false);
      expect(changes, [false]);
      await tester.pumpWidget(app(false, enabled: false));
      await tester.pump();
      expect(calls.last.arguments, containsPair('value', false));
      expect(calls.last.arguments, containsPair('enabled', false));
      await change(true);
      expect(changes, [false]);
      await tester.pumpWidget(app(true));
      await tester.pump();
      expect(calls.last.arguments, containsPair('value', true));
      await tester.pumpWidget(const SizedBox());
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    },
  );
}
