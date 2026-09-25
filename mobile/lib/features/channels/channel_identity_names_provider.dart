import 'dart:collection';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/identity_names/identity_names.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';

/// Contextual identity labels for one channel, thread, or DM.
///
/// The comparison context is the channel's members; a referenced non-member
/// (historical author, outside mention) is compared against the members plus
/// itself. Watch one label with
/// `ref.watch(channelIdentityNamesProvider(id).select((n) => n.labelFor(pk)))`.
final channelIdentityNamesProvider = Provider.autoDispose
    .family<IdentityNames, String>((ref, channelId) {
      final sources = ref.watch(identityNameSourcesProvider);
      final loaded = ref.watch(channelMembersProvider(channelId)).asData?.value;
      final members =
          loaded ??
          (ref.watch(channelsProvider).asData == null
              ? const <ChannelMember>[]
              : ref
                    .read(channelsProvider.notifier)
                    .cachedMembersForChannel(channelId));
      final names = sources.scope(
        [for (final member in members) member.pubkey],
        agentPubkeys: ref.watch(agentMentionPubkeysProvider(channelId)),
        fallbackNames: {
          for (final member in members) member.pubkey: ?member.displayName,
        },
      );
      loadIdentityNameOwners(ref, sources, names.candidates);
      return names;
    });

/// Watches one identity's contextual label in [channelId].
String watchChannelIdentityLabel(
  WidgetRef ref,
  String channelId,
  String pubkey,
) => ref.watch(
  channelIdentityNamesProvider(
    channelId,
  ).select((names) => names.labelFor(pubkey)),
);

/// Watches contextual labels for [pubkeys] in [channelId], keyed by
/// lowercase pubkey. Rebuilds only when one of these labels changes.
Map<String, String> watchChannelIdentityLabels(
  WidgetRef ref,
  String channelId,
  Iterable<String> pubkeys,
) {
  final keys = {for (final pubkey in pubkeys) pubkey.toLowerCase()};
  if (keys.isEmpty) return const {};
  return ref.watch(
    channelIdentityNamesProvider(channelId).select(
      (names) => _LabelMap({for (final key in keys) key: names.labelFor(key)}),
    ),
  );
}

class _LabelMap extends UnmodifiableMapView<String, String> {
  _LabelMap(super.labels);

  @override
  bool operator ==(Object other) =>
      other is Map<String, String> && mapEquals(this, other);

  @override
  int get hashCode => Object.hashAllUnordered(
    entries.map((entry) => Object.hash(entry.key, entry.value)),
  );
}
