import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'community.dart';

/// A one-shot request to reveal an imported community through its avatar.
final pairedCommunityLandingProvider =
    NotifierProvider<PairedCommunityLanding, Community?>(
      PairedCommunityLanding.new,
    );

/// Keeps the handoff alive while authentication replaces the onboarding page.
class PairedCommunityLanding extends Notifier<Community?> {
  @override
  Community? build() => null;

  /// Requests the shared loading and avatar flight after a successful import.
  void request(Community community) => state = community;

  /// Consumes the request once the destination page owns the transition.
  void clear() => state = null;
}
