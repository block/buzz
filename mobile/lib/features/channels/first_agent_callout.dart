import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_sync.dart';
import '../../shared/theme/theme_provider.dart';

/// The first-agent tip is dismissed once per account and community.
String firstAgentCalloutKey(MeSync scope) =>
    'buzz-first-agent-callout.v1:${Uri.encodeComponent(scope.community)}:'
    '${scope.pubkey}';

/// Whether this account has already dismissed the tip in this community.
/// Without an identity there is nothing to scope it to, so it stays hidden.
final firstAgentCalloutSeenProvider = Provider.autoDispose<bool>((ref) {
  final scope = ref.watch(meSyncProvider);
  if (scope.config.nsec == null) return true;
  return ref.watch(savedPrefsProvider).getBool(firstAgentCalloutKey(scope)) ??
      false;
});

/// Hides the tip for good. Preferences update in memory at once, so a failed
/// disk write only means the tip may return on the next launch.
void dismissFirstAgentCallout(WidgetRef ref) {
  final scope = ref.read(meSyncProvider);
  if (scope.config.nsec == null) return;
  unawaited(
    ref
        .read(savedPrefsProvider)
        .setBool(firstAgentCalloutKey(scope), true)
        .catchError((Object _) => false),
  );
  ref.invalidate(firstAgentCalloutSeenProvider);
}
