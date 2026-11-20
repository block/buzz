import 'package:buzz/features/channels/bestie_transcription_surface.dart';
import 'package:buzz/features/home/messages_navigation_bar.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/agent_navigation_overlay.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  Widget app(Widget child) => ProviderScope(
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(body: Center(child: child)),
    ),
  );

  Widget bar(Axis axis, {Widget? agent}) => MessagesNavigationBar(
    axis: axis,
    onSearch: () {},
    creationActions: const [],
    agent:
        agent ??
        Builder(
          builder: (context) => SizedBox.square(
            key: const ValueKey('agent'),
            dimension: AgentNavigationOverlay.controlSizeOf(context),
          ),
        ),
  );

  testWidgets("Duo's vertical rail group uses the native rail control size", (
    tester,
  ) async {
    await tester.pumpWidget(app(bar(Axis.vertical)));
    final group = find.byType(MessagesNavigationBar);
    // Three 48-point controls with the rail's 12-point gaps.
    expect(tester.getSize(group).height, 48 * 3 + 12 * 2);
    final create = tester.widget<IosMessageCreateButton>(
      find.byType(IosMessageCreateButton),
    );
    expect(create.width, 48);
    expect(create.height, 168);
    expect(create.navigationControlSize, 48);
    expect(create.navigationGap, 12);
    final search = find.ancestor(
      of: find.byTooltip('Search'),
      matching: find.byType(SizedBox),
    );
    expect(tester.getSize(search.first), const Size.square(48));
    final agent = find.byKey(const ValueKey('agent'));
    expect(tester.getSize(agent), const Size.square(48));
    expect(tester.getBottomRight(agent), tester.getBottomRight(group));
    expect(tester.getTopRight(search.first), tester.getTopRight(group));
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('the horizontal group keeps its 56-point controls', (
    tester,
  ) async {
    await tester.pumpWidget(app(bar(Axis.horizontal)));
    expect(tester.getSize(find.byType(MessagesNavigationBar)).height, 56);
    final create = tester.widget<IosMessageCreateButton>(
      find.byType(IosMessageCreateButton),
    );
    expect(create.navigationControlSize, 56);
    expect(create.navigationGap, Grid.xs);
    expect(
      tester.getSize(find.byKey(const ValueKey('agent'))),
      const Size.square(56),
    );
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('the Bestie recording pill keeps the rail control height', (
    tester,
  ) async {
    Widget surface({required bool recording}) => BestieTranscriptionSurface(
      enabled: true,
      recording: recording,
      finalizing: false,
      sending: false,
      restoring: false,
      restoreFailed: false,
      hasPending: false,
      elapsed: Duration.zero,
      samples: const [],
      error: null,
      cancelled: ValueNotifier(false),
      start: () async {},
      finish: () async {},
      cancel: () async {},
      retry: () async {},
      restore: () async {},
      discardPending: () async {},
      child: const SizedBox.expand(key: ValueKey('bestie')),
    );
    final pill = find.byKey(const ValueKey('default-agent-voice-pill'));
    for (final (axis, size) in [
      (Axis.vertical, 48.0),
      (Axis.horizontal, 56.0),
    ]) {
      for (final recording in [false, true]) {
        await tester.pumpWidget(
          app(bar(axis, agent: surface(recording: recording))),
        );
        await tester.pumpAndSettle();
        expect(tester.getSize(pill).height, size);
        expect(tester.getSize(pill).width, recording ? 256 : size);
        expect(
          tester.getSize(find.byKey(const ValueKey('bestie'))),
          Size.square(size),
        );
      }
    }
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
