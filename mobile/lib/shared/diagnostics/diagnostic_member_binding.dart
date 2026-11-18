import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay_provider.dart';
import 'diagnostic_reporter.dart';

/// Keeps later reports attributed to the signed-in member: set when a session
/// starts, changed when the account switches and cleared on sign-out.
final diagnosticMemberProvider = Provider<void>((ref) {
  ref.watch(diagnosticReporterProvider).setMember(ref.watch(myPubkeyProvider));
});

/// Holds [diagnosticMemberProvider] for the life of the app.
class DiagnosticMemberBinding extends ConsumerWidget {
  const DiagnosticMemberBinding({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    ref.watch(diagnosticMemberProvider);
    return child;
  }
}
