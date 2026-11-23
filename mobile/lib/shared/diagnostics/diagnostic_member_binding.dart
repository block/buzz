import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../hosted/builderlab_build.dart';
import '../http/app_http_client.dart';
import '../relay/relay_provider.dart';
import 'diagnostic_reporter.dart';

/// Keeps later reports attributed to the signed-in member: set when a session
/// starts, changed when the account switches and cleared on sign-out.
final diagnosticMemberProvider = Provider<void>((ref) {
  ref.watch(diagnosticReporterProvider).setMember(ref.watch(myPubkeyProvider));
});

/// Marks the active relay's host, and the build's own services, as first
/// party, so HTTP counts name them ([httpBackend]). A relay stays marked after
/// a switch away, for the rest of the run.
final diagnosticBackendsProvider = Provider<void>((ref) {
  void add(String url) {
    final host = Uri.tryParse(url)?.host.toLowerCase() ?? '';
    if (host.isNotEmpty) appHttpFirstPartyHosts.add(host);
  }

  add(ref.watch(relayConfigProvider.select((config) => config.baseUrl)));
  add(Env.pushGatewayUrl);
  if (BuilderlabBuild.origin case final origin?) add(origin.toString());
});

/// Holds [diagnosticMemberProvider] and [diagnosticBackendsProvider] for the
/// life of the app.
class DiagnosticMemberBinding extends ConsumerWidget {
  const DiagnosticMemberBinding({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    ref
      ..watch(diagnosticMemberProvider)
      ..watch(diagnosticBackendsProvider);
    return child;
  }
}
