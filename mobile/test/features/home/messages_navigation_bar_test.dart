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

  Widget bar({Widget? agent}) => MessagesNavigationBar(
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

  testWidgets('the horizontal group keeps its 56-point controls', (
    tester,
  ) async {
    await tester.pumpWidget(app(bar()));
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

  testWidgets('the Bestie recording pill follows the control size', (
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
    // The Duo rail hosts the surface in a 48-point overlay; the bar keeps 56.
    Widget hosted(double size, bool recording) => app(
      size == 48
          ? AgentNavigationOverlay(
              active: ValueNotifier(false),
              controlSize: size,
              child: surface(recording: recording),
            )
          : bar(agent: surface(recording: recording)),
    );
    for (final size in [48.0, 56.0]) {
      for (final recording in [false, true]) {
        await tester.pumpWidget(hosted(size, recording));
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
