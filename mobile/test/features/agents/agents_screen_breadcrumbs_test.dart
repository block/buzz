import 'package:buzz/diagnostic_screens.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/diagnostics/diagnostic_navigation_observer.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets('creating an agent is its own screen in the trail', (
    tester,
  ) async {
    final reporter = _RecordingReporter();
    final h = await AgentsHarness.create();
    await h.open(
      tester,
      directory: const [],
      overrides: [
        diagnosticReporterProvider.overrideWithValue(reporter),
        diagnosticScreenClassifierProvider.overrideWithValue(
          diagnosticScreenOf,
        ),
      ],
    );
    expect(reporter.screens, [DiagnosticScreen.agents]);

    // The Agents sheet pushes the create form on its own navigator.
    final agents = Navigator.of(tester.element(find.byType(AgentsPage)));
    agents.push(
      MaterialPageRoute<void>(
        builder: (_) => CreateAgentPage(
          controller: h.controller,
          community: h.community,
          onCreated: () {},
        ),
      ),
    );
    await tester.pumpAndSettle();
    agents.pop();
    await tester.pumpAndSettle();

    expect(reporter.screens, [
      DiagnosticScreen.agents,
      DiagnosticScreen.createAgent,
      DiagnosticScreen.agents,
    ]);
  });
}

class _RecordingReporter extends NoopDiagnosticReporter {
  final screens = <DiagnosticScreen>[];

  @override
  bool get reporting => true;

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) {
    if (breadcrumb case ScreenBreadcrumb(:final screen)) screens.add(screen);
  }
}
