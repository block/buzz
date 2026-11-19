part of '../channel_detail_page.dart';

class _HuddleCardSummary extends HookConsumerWidget {
  const _HuddleCardSummary({
    required this.message,
    required this.allMessages,
    required this.parentChannelId,
    required this.now,
    required this.finished,
  });

  final TimelineMessage message;
  final List<TimelineMessage> allMessages;
  final String parentChannelId;
  final DateTime now;
  final bool finished;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final room = message.systemEvent!.ephemeralChannelId!;
    final session = ref.watch(huddleSessionProvider);
    final lifecycle =
        ref.watch(huddleLifecycleProvider(parentChannelId)).value ??
        const <NostrEvent>[];
    final relay = ref.watch(relayConfigProvider).baseUrl;
    final authority = ref.watch(relayAuthorProvider(relay)).value;
    final events =
        lifecycle.where((event) {
          if (event.getTagValue('h') != parentChannelId ||
              event.createdAt < message.createdAt) {
            return false;
          }
          try {
            final body = jsonDecode(event.content);
            return body is Map && body['ephemeral_channel_id'] == room;
          } catch (_) {
            return false;
          }
        }).toList()..sort((a, b) {
          final time = a.createdAt.compareTo(b.createdAt);
          if (time != 0) return time;
          int phase(int kind) => kind == EventKind.huddleStarted
              ? 0
              : kind == EventKind.huddleEnded
              ? 2
              : 1;
          final kind = phase(a.kind).compareTo(phase(b.kind));
          if (kind != 0) return kind;
          final left = jsonDecode(a.content) as Map;
          final right = jsonDecode(b.content) as Map;
          final ar = left['roster_revision'];
          final br = right['roster_revision'];
          if (ar is int && br is int && ar != br) return ar.compareTo(br);
          return a.id.compareTo(b.id);
        });
    int? endedAt;
    final people = <String>{};
    final admissions = <String, Map<String, NostrEvent>>{};
    String? generation;

    for (final event in events) {
      if (event.kind == EventKind.huddleEnded &&
          (event.pubkey == authority || event.pubkey == message.pubkey)) {
        endedAt ??= event.createdAt;
      } else if (endedAt == null &&
          authority != null &&
          event.pubkey == authority) {
        final key = event.getTagValue('p');
        if (key == null || !RegExp(r'^[0-9a-f]{64}$').hasMatch(key)) continue;
        if (event.kind != EventKind.huddleParticipantJoined &&
            event.kind != EventKind.huddleParticipantLeft) {
          continue;
        }
        final body = jsonDecode(event.content) as Map;
        final nextGeneration = body['generation'];
        if (nextGeneration is String) {
          if (generation != null && generation != nextGeneration) {
            if (event.kind == EventKind.huddleParticipantLeft) continue;
            admissions.clear();
          }
          generation = nextGeneration;
        }
        final rawAdmission = body['admission_id'];
        final admission = rawAdmission is String ? rawAdmission : 'legacy';
        final roster = admissions.putIfAbsent(key, () => {});
        final previous = roster[admission];
        if (previous != null) {
          final previousBody = jsonDecode(previous.content) as Map;
          final revision = body['roster_revision'];
          final previousRevision = previousBody['roster_revision'];
          if (revision is int &&
              previousRevision is int &&
              revision < previousRevision) {
            continue;
          }
        }
        roster[admission] = event;
      }
    }
    for (final entry in admissions.entries) {
      if (entry.value.values.any(
        (event) => event.kind == EventKind.huddleParticipantJoined,
      )) {
        people.add(entry.key);
      }
    }
    for (final row in allMessages) {
      if (row.systemEvent?.type == SystemEventType.huddleEnded &&
          row.systemEvent?.ephemeralChannelId == room &&
          row.createdAt >= message.createdAt) {
        endedAt = endedAt == null ? row.createdAt : min(endedAt, row.createdAt);
      }
    }
    if (session.ephemeralChannelId == room && session.isConnected) {
      people
        ..clear()
        ..addAll(session.participantPubkeys);
      if (session.currentPubkey case final key?) people.add(key);
    }
    final pubkeys = people.toList();
    final profiles = ref.watch(userCacheProvider);
    useEffect(() {
      if (pubkeys.isNotEmpty) {
        Future.microtask(() {
          if (context.mounted) {
            ref.read(userCacheProvider.notifier).preload(pubkeys);
          }
        });
      }
      return null;
    }, [pubkeys.join(':')]);
    String name(String key) =>
        profiles[key]?.displayName ?? key.substring(0, min(8, key.length));
    final seconds = max(
      0,
      (endedAt ?? now.millisecondsSinceEpoch ~/ 1000) - message.createdAt,
    );
    final visible = pubkeys.length > 4
        ? pubkeys.sublist(pubkeys.length - 4)
        : pubkeys;
    final overflow = pubkeys.length - visible.length;
    return Wrap(
      spacing: Grid.xxs,
      runSpacing: Grid.half,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        if (!finished || endedAt != null)
          Semantics(
            label: '${seconds ~/ 60} minutes ${seconds % 60} seconds',
            child: ExcludeSemantics(
              child: Text(
                '${seconds ~/ 60}:${(seconds % 60).toString().padLeft(2, '0')}',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.huddleCardSecondary,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
            ),
          ),
        if (visible.isNotEmpty)
          Semantics(
            container: true,
            label: 'In this Huddle: ${pubkeys.map(name).join(', ')}',
            child: ExcludeSemantics(
              child: SizedBox(
                width: 20 + (visible.length - 1 + (overflow > 0 ? 1 : 0)) * 14,
                height: 20,
                child: Stack(
                  children: [
                    for (var index = 0; index < visible.length; index++)
                      PositionedDirectional(
                        start: index * 14.0,
                        child: AvatarImage(
                          key: ValueKey('huddle-card-avatar-${visible[index]}'),
                          imageUrl: profiles[visible[index]]?.avatarUrl,
                          radius: 10,
                          backgroundColor: context.huddleCardControl,
                          fallback: Text(
                            name(visible[index]).characters.firstOrNull ?? '?',
                            style: context.textTheme.labelSmall,
                          ),
                        ),
                      ),
                    if (overflow > 0)
                      PositionedDirectional(
                        start: visible.length * 14.0,
                        child: CircleAvatar(
                          radius: 10,
                          backgroundColor: context.huddleCardControl,
                          child: Text(
                            '+$overflow',
                            style: context.textTheme.labelSmall,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// Resolved desktop HuddleCard tokens (surface-panel, border-standard,
/// interaction-fill, text-standard/subtle and affordance/text-danger).
/// Kept local because mobile's generic Material roles have different values.
extension _HuddleCardTheme on BuildContext {
  bool get _huddleDark => colors.brightness == Brightness.dark;
  Color get huddleCardSurface =>
      _huddleDark ? const Color(0xFF1A1A1A) : const Color(0xFFFFFFFF);
  Color get huddleCardBorder =>
      _huddleDark ? const Color(0xFF333333) : const Color(0xFFE8E8E8);
  Color get huddleCardControl =>
      _huddleDark ? const Color(0xFF232323) : const Color(0xFFF5F5F6);
  Color get huddleCardText =>
      _huddleDark ? const Color(0xFFFFFFFF) : const Color(0xFF0F0F0F);
  Color get huddleCardSecondary =>
      _huddleDark ? const Color(0xFFC7C7C7) : const Color(0xFF525252);
  Color get huddleCardEndedSurface =>
      _huddleDark ? const Color(0xFF3B1219) : const Color(0xFFFEEBEC);
  Color get huddleCardEndedText =>
      _huddleDark ? const Color(0xFFFFD1D9) : const Color(0xFFA9252C);
}
