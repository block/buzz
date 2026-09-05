import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/ios_native_navigation_shell.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'keeps one native shell mounted and updates its navigation item',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      late IosNativeNavigationShellController controller;
      final owner = Object();
      final calls = <MethodCall>[];
      var backCount = 0;
      var titleCount = 0;
      var huddleCount = 0;
      var membersCount = 0;
      var moreCount = 0;

      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: IosNativeNavigationShellHost(
            child: Builder(
              builder: (context) {
                controller = IosNativeNavigationShellController.maybeOf(
                  context,
                )!;
                return const SizedBox.expand();
              },
            ),
          ),
        ),
      );

      final nativeShell = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(nativeShell.viewType, IosNativeNavigationShellHost.viewType);
      expect(
        nativeShell.hitTestBehavior,
        PlatformViewHitTestBehavior.transparent,
      );
      const viewId = 42;
      final channel = MethodChannel(
        '${IosNativeNavigationShellHost.viewType}/$viewId',
      );
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
      nativeShell.onPlatformViewCreated!(viewId);
      await tester.pump();

      controller.show(
        owner,
        IosNativeNavigationConfiguration(
          title: 'general',
          subtitle: '3 members',
          semanticLabel: 'Open settings for general, 3 members',
          foregroundColor: Colors.blue,
          brightness: Brightness.light,
          systemIconName: 'number',
          onBack: () => backCount++,
          onTitle: () => titleCount++,
          showsHuddle: true,
          onHuddle: () => huddleCount++,
          huddleLabel: 'Start Huddle',
          onMembers: () => membersCount++,
          onMore: () => moreCount++,
        ),
      );
      await tester.pump();

      expect(
        tester.widget<UiKitView>(find.byType(UiKitView)).hitTestBehavior,
        PlatformViewHitTestBehavior.opaque,
      );

      final visibleCall = calls.last;
      expect(visibleCall.method, 'setNavigation');
      expect(visibleCall.arguments, containsPair('title', 'general'));
      expect(visibleCall.arguments, containsPair('showsHuddle', true));
      expect(visibleCall.arguments, containsPair('showsMembers', true));
      expect(visibleCall.arguments, containsPair('showsMore', true));

      for (final method in ['back', 'title', 'huddle', 'members', 'more']) {
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          channel.name,
          channel.codec.encodeMethodCall(MethodCall(method)),
          (_) {},
        );
      }
      expect(backCount, 1);
      expect(titleCount, 1);
      expect(huddleCount, 1);
      expect(membersCount, 1);
      expect(moreCount, 1);

      controller.hide(owner);
      await tester.pump();
      expect(calls.last.arguments, containsPair('visible', false));
      expect(
        tester.widget<UiKitView>(find.byType(UiKitView)).hitTestBehavior,
        PlatformViewHitTestBehavior.transparent,
      );
      debugDefaultTargetPlatformOverride = null;
    },
  );
}
