import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'me_sync.dart';

/// Optional Me placement must not make ordinary search results inaccessible.
final meSearchPlacementProvider = FutureProvider.autoDispose
    .family<bool, String>((ref, channelId) async {
      final state = ref.read(meGroupsProvider);
      final cached = state.value;
      if (cached != null) return cached.channels.contains(channelId);
      if (state.hasError) return false;
      try {
        final groups = await ref
            .read(meGroupsProvider.future)
            .timeout(const Duration(seconds: 2));
        return groups.channels.contains(channelId);
      } catch (_) {
        return false;
      }
    });
