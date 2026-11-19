import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../deeplink/deep_link.dart';

/// Opens the editable agent-creation draft for a `buzz://agent/new` link.
///
/// Supplied by the app composition root so `features/channels` can route the
/// link without importing `features/agents`. Null when unavailable.
final agentCreationSurfaceProvider =
    Provider<Future<void> Function(BuildContext, AgentCreationDeepLink)?>(
      (ref) => null,
    );
