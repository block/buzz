import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('shows icon-only navigation and an aligned quick action', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const HomePage(settingsPageBuilder: _buildSettingsPage),
        ),
      ),
    );
    await tester.pump();

    expect(find.text('Home'), findsNothing);
    expect(find.text('Activity'), findsNothing);
    expect(find.text('Search'), findsNothing);
    expect(find.bySemanticsLabel('Home'), findsOneWidget);
    expect(find.bySemanticsLabel('Activity'), findsOneWidget);
    expect(find.bySemanticsLabel('Search'), findsOneWidget);

    final quickAction = find.byTooltip('Create or start conversation');
    expect(quickAction, findsOneWidget);
    final launcherSize = tester.getSize(
      find.byType(ChannelQuickActionsLauncher),
    );
    expect(launcherSize.width, 800);
    expect(launcherSize.height, greaterThan(0));
    final motionRect = tester.getRect(
      find.byKey(const Key('channel-quick-actions-motion')),
    );
    expect(motionRect.width, const Size.square(56).width);
    expect(motionRect.left, greaterThanOrEqualTo(0));
    expect(tester.getSize(quickAction), const Size.square(56));
    final quickActionRect = tester.getRect(quickAction);
    expect(quickActionRect.left, greaterThanOrEqualTo(0));
    expect(quickActionRect.top, greaterThanOrEqualTo(0));
    expect(quickActionRect.right, lessThanOrEqualTo(800));
    expect(quickActionRect.bottom, lessThanOrEqualTo(600));
    final homeDestinationRect = tester.getRect(find.bySemanticsLabel('Home'));
    expect(
      quickActionRect.center.dy,
      closeTo(homeDestinationRect.center.dy, 0.01),
    );
  });

  testWidgets('gives selection haptics only when the tab changes', (
    tester,
  ) async {
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            hapticCalls.add(call);
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const HomePage(settingsPageBuilder: _buildSettingsPage),
        ),
      ),
    );
    await tester.pump();

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    expect(hapticCalls, isEmpty);

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));

    await tester.tap(find.byTooltip('Search'));
    await tester.pump();
    expect(hapticCalls, hasLength(2));
  });

  testWidgets('scales and fades the quick action as tabs change', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const HomePage(settingsPageBuilder: _buildSettingsPage),
        ),
      ),
    );
    await tester.pump();

    double scale() => tester
        .widget<Transform>(find.byKey(const Key('channel-quick-actions-scale')))
        .transform
        .storage
        .first;
    double opacity() => tester
        .widget<Opacity>(find.byKey(const Key('channel-quick-actions-opacity')))
        .opacity;

    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(0.8, 0.001));
    expect(opacity(), closeTo(0, 0.001));

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));
  });
}

Widget _buildSettingsPage(BuildContext context) => const SizedBox.shrink();
