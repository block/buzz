import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../community/community.dart';
import '../community/community_storage.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_client.dart' show RelayException;
import '../relay/relay_http_query_client.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_query_response.dart';
import '../relay/relay_session.dart' show buildNip98AuthHeader;
import '../utils/string_utils.dart';
import 'notification_body.dart';
import 'push_wake_eligibility.dart';
import 'shown_message_store.dart';

/// Most notifications one wake returns. The newest are kept, so a phone that
/// was offline for a day gets a screenful, not hundreds.
const pushWakeMaxNotifications = 20;

/// Most notifications one conversation gets from one wake. Android's
/// `MessagingStyle` keeps at most 25 messages and the shade is unreadable well
/// before that, so a busy conversation shows its newest 8 and the rest are
/// older than anything the user can see. Messages beyond the cap are still
/// recorded as shown, so a later wake does not bring them back.
const pushWakeMaxPerConversation = 8;

/// How a notification is categorized, which is the Android channel the user
/// may have silenced. A message in a direct-message channel is a
/// [directMessage] even when it also mentions the user.
enum PushWakeCategory { directMessage, mention }

/// Where tapping a notification goes. These are the fields of the app's
/// message deep link.
class PushWakeTarget {
  const PushWakeTarget({
    required this.communityId,
    required this.channelId,
    required this.eventId,
    this.threadRootId,
  });

  final String communityId;
  final String channelId;
  final String eventId;
  final String? threadRootId;

  Map<String, String> toMap() => {
    'communityId': communityId,
    'channelId': channelId,
    'eventId': eventId,
    'threadRootId': ?threadRootId,
  };

  @override
  bool operator ==(Object other) =>
      other is PushWakeTarget &&
      other.communityId == communityId &&
      other.channelId == channelId &&
      other.eventId == eventId &&
      other.threadRootId == threadRootId;

  @override
  int get hashCode =>
      Object.hash(communityId, channelId, eventId, threadRootId);
}

/// One message to notify about.
class PushWakeNotification {
  const PushWakeNotification({
    required this.eventId,
    required this.createdAt,
    required this.title,
    required this.body,
    required this.conversationId,
    this.conversationTitle,
    required this.category,
    required this.target,
  });

  final String eventId;
  final DateTime createdAt;

  /// The sender's name.
  final String title;

  /// Plain text from [notificationBody].
  final String body;

  /// Stable per conversation and community, for grouping.
  final String conversationId;

  /// `#name` for a channel, null for a direct message.
  final String? conversationTitle;
  final PushWakeCategory category;
  final PushWakeTarget target;

  PushWakeNotification named({String? title, String? conversationTitle}) =>
      PushWakeNotification(
        eventId: eventId,
        createdAt: createdAt,
        title: title ?? this.title,
        body: body,
        conversationId: conversationId,
        conversationTitle: conversationTitle,
        category: category,
        target: target,
      );
}

/// What a wake found.
class PushWakeResult {
  const PushWakeResult({
    required this.notifications,
    this.failedCommunityIds = const [],
  });

  /// Every eligible unshown message, newest first, within the caps.
  final List<PushWakeNotification> notifications;

  /// Communities whose messages could not be read in time. Their messages are
  /// not recorded as shown, so the next wake retries them.
  final List<String> failedCommunityIds;

  /// Whether the wake could not look everywhere. With no [notifications] the
  /// caller shows its generic notification, so the wake is never silent.
  bool get incomplete => failedCommunityIds.isNotEmpty;
}

/// Sorts newest first and applies [pushWakeMaxPerConversation] and
/// [pushWakeMaxNotifications].
List<PushWakeNotification> capPushWakeNotifications(
  Iterable<PushWakeNotification> notifications,
) {
  final newestFirst = notifications.toList()
    ..sort(
      (a, b) => a.createdAt != b.createdAt
          ? b.createdAt.compareTo(a.createdAt)
          : a.eventId.compareTo(b.eventId),
    );
  final perConversation = <String, int>{};
  final kept = <PushWakeNotification>[];
  for (final notification in newestFirst) {
    final count = perConversation.update(
      notification.conversationId,
      (count) => count + 1,
      ifAbsent: () => 1,
    );
    if (count > pushWakeMaxPerConversation) continue;
    kept.add(notification);
    if (kept.length == pushWakeMaxNotifications) break;
  }
  return kept;
}

/// Time limits of a wake. The native watchdog ends the work at about 8 s
/// whatever Dart does, so the whole wake stays well inside that, and one slow
/// relay can't use the budget the others need.
class PushWakeTimeouts {
  const PushWakeTimeouts({
    this.request = const Duration(seconds: 3),
    this.metadata = const Duration(milliseconds: 1500),
    this.total = const Duration(seconds: 5),
  });

  /// Longest one HTTP request to a relay may take, including its body.
  final Duration request;

  /// Longest the lookup of sender and channel names may take. It comes after
  /// the messages are fetched and recorded, so [request] plus this stays
  /// inside [total], and a slow lookup costs only the names.
  final Duration metadata;

  /// Longest the whole wake may take. At this point it returns what it has.
  final Duration total;
}

/// Reads the configured communities and returns every eligible message not
/// yet shown, claiming each in [store] before returning it.
///
/// It does what the iOS notification extension's resolver does, for a wake
/// that carries no content: sign in to each relay, fetch what is newer than
/// the last shown message, apply the subscription's eligibility rules
/// (self-authored, muted channels, hellthreads) and build title, body and
/// target for each message.
///
/// It runs headless, with no UI and no Riverpod state, so it is safe to run
/// in a background engine. Callers run it one at a time per process; see
/// `PushWakeRunner`.
Future<PushWakeResult> resolvePushWake({
  required ShownMessageStore store,
  Future<List<Community>> Function()? loadCommunities,
  http.Client? httpClient,
  PushWakeTimeouts timeouts = const PushWakeTimeouts(),
  DateTime Function()? now,
}) async {
  final clock = now ?? DateTime.now;
  final communities = await (loadCommunities ?? CommunityStorage().loadAll)();
  final eligible = [
    for (final community in communities)
      if (community.pushNotificationsEnabled &&
          (community.nsec?.isNotEmpty ?? false) &&
          community.pushSubscriptionState.authoritative.isNotEmpty)
        community,
  ];
  final client = RelayHttpQueryClient(client: httpClient);
  final run = _WakeRun();
  final tasks = [
    for (final community in eligible)
      _resolveCommunity(
        community,
        run: run,
        store: store,
        client: client,
        timeouts: timeouts,
        clock: clock,
      ).catchError((Object _) {
        run.failed.add(community.id);
      }),
  ];
  final deadline = Completer<void>();
  final timer = Timer(timeouts.total, deadline.complete);
  try {
    await Future.any([Future.wait(tasks), deadline.future]);
  } finally {
    timer.cancel();
  }
  // From here no task may start claiming. A claim already running finishes,
  // so no message is recorded as shown without being returned.
  run.closed = true;
  await Future.wait(run.claims);
  for (final community in eligible) {
    if (!run.done.contains(community.id)) run.failed.add(community.id);
  }
  // The caller may be in a long-lived process, and a request that outlived
  // the budget must not keep holding the connection.
  client.close();
  return PushWakeResult(
    notifications: capPushWakeNotifications(run.notifications),
    failedCommunityIds: run.failed.toSet().toList()..sort(),
  );
}

class _WakeRun {
  final notifications = <PushWakeNotification>[];
  final failed = <String>[];
  final done = <String>{};
  final claims = <Future<void>>[];
  bool closed = false;
}

Future<void> _resolveCommunity(
  Community community, {
  required _WakeRun run,
  required ShownMessageStore store,
  required RelayHttpQueryClient client,
  required PushWakeTimeouts timeouts,
  required DateTime Function() clock,
}) async {
  final nsec = community.nsec!;
  final mine = (community.pubkey ?? pubkeyFromNsec(nsec))?.toLowerCase();
  if (mine == null) throw StateError('Community has no usable key');
  final subscriptions = community.pushSubscriptionState.authoritative;
  final config = RelayConfig(baseUrl: community.relayUrl, nsec: nsec);
  final url = Uri.parse(config.baseUrl).resolve('/query');

  Future<List<NostrEvent>> query(
    List<Map<String, dynamic>> filters, {
    required Duration timeout,
  }) async {
    final body = utf8.encode(jsonEncode(filters));
    final response = await client.post(
      url,
      headers: {
        'Authorization': buildNip98AuthHeader(
          method: 'POST',
          url: url.toString(),
          bodyBytes: body,
          nsec: nsec,
          pubkey: mine,
        ),
        'Content-Type': 'application/json',
      },
      body: body,
      timeout: timeout,
    );
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw RelayException(response.statusCode, '');
    }
    return parseRelayQueryEvents(
      response.bodyBytes,
      response.headers['content-type'],
    );
  }

  final newest = await store.newestShown(community.id);
  // Without a record there is no history to bound the fetch, and the message
  // that caused this wake is recent, so look back one reorder window only.
  final since =
      (newest ?? clock()).millisecondsSinceEpoch ~/ 1000 -
      shownMessageReorderWindow.inSeconds;
  final fetched = await query([
    for (final subscription in subscriptions)
      {...subscription.filter.toJson(), 'since': since, 'limit': 50},
  ], timeout: timeouts.request);

  final selected = <({NostrEvent event, PushWakeCategory category})>[];
  final seen = <String>{};
  for (final event in fetched) {
    if (!seen.add(event.id) ||
        event.pubkey.toLowerCase() == mine ||
        !_hasValidIdAndSignature(event)) {
      continue;
    }
    final matching = [
      for (final subscription in subscriptions)
        if (pushSubscriptionMatches(subscription, event)) subscription,
    ];
    if (matching.isEmpty || event.channelId == null) continue;
    selected.add((
      event: event,
      category: matching.any((s) => s.filter.hTags != null)
          ? PushWakeCategory.directMessage
          : PushWakeCategory.mention,
    ));
  }
  selected.sort(
    (a, b) => a.event.createdAt != b.event.createdAt
        ? b.event.createdAt.compareTo(a.event.createdAt)
        : a.event.id.compareTo(b.event.id),
  );

  if (run.closed) return;
  final work = () async {
    // Claim first, so a message already shown costs no further requests: the
    // fetch overlaps the last shown messages by the reorder window, so most
    // wakes see messages they have already shown.
    final claimed = <PushWakeNotification>[];
    final senders = <String, String>{};
    for (final (:event, :category) in selected) {
      final createdAt = DateTime.fromMillisecondsSinceEpoch(
        event.createdAt * 1000,
        isUtc: true,
      );
      if (!await store.claim(community.id, event.id, createdAt)) continue;
      final body = notificationBody(event.content);
      // A message with no text (only markup) has nothing to show, but it is
      // recorded so it is not looked at again.
      if (body.isEmpty) continue;
      final channelId = event.channelId!;
      senders[event.id] = event.pubkey.toLowerCase();
      claimed.add(
        PushWakeNotification(
          eventId: event.id,
          createdAt: createdAt,
          title: shortPubkey(event.pubkey),
          body: body,
          conversationId: '${community.id}:$channelId',
          category: category,
          target: PushWakeTarget(
            communityId: community.id,
            channelId: channelId,
            eventId: event.id,
            threadRootId: event.threadReference.rootId,
          ),
        ),
      );
    }
    // Names are presentation, and only the messages that will show need
    // them. A failed or slow lookup leaves a short key as the title instead of
    // hiding a message that is already recorded as shown.
    final kept = capPushWakeNotifications(claimed);
    var names = <String, String>{};
    var channels = <String, String>{};
    if (kept.isNotEmpty) {
      try {
        final people = {for (final n in kept) senders[n.eventId]!}.toList();
        final channelIds = {for (final n in kept) n.target.channelId}.toList();
        final metadata = await query([
          {
            'kinds': [EventKind.profile],
            'authors': people,
            'limit': people.length,
          },
          {
            'kinds': [39000],
            '#d': channelIds,
            'limit': channelIds.length,
          },
        ], timeout: timeouts.metadata);
        names = _profileNames(metadata);
        channels = _channelNames(metadata);
      } on Object {
        // Keep the fallback labels.
      }
    }
    run.notifications.addAll([
      for (final n in kept)
        n.named(
          title: names[senders[n.eventId]],
          conversationTitle: n.category == PushWakeCategory.directMessage
              ? null
              : channels[n.target.channelId],
        ),
    ]);
    run.done.add(community.id);
  }();
  run.claims.add(work);
  await work;
}

bool _hasValidIdAndSignature(NostrEvent event) {
  try {
    final signed = nostr.Event.fromMap(event.toJson(), verify: false);
    return signed.getEventId() == event.id &&
        nostr.Schnorr.verify(
          publicKey: event.pubkey,
          message: event.id,
          signature: event.sig,
        );
  } on Object {
    return false;
  }
}

Map<String, String> _profileNames(List<NostrEvent> events) {
  final newest = <String, NostrEvent>{};
  for (final event in events) {
    if (event.kind != EventKind.profile) continue;
    final existing = newest[event.pubkey.toLowerCase()];
    if (existing == null || event.createdAt > existing.createdAt) {
      newest[event.pubkey.toLowerCase()] = event;
    }
  }
  final names = <String, String>{};
  for (final MapEntry(:key, :value) in newest.entries) {
    try {
      final meta = jsonDecode(value.content);
      if (meta is! Map) continue;
      final name = (meta['display_name'] ?? meta['name'])?.toString().trim();
      if (name != null && name.isNotEmpty) {
        names[key] = name.length > 100 ? name.substring(0, 100) : name;
      }
    } on FormatException {
      continue;
    }
  }
  return names;
}

Map<String, String> _channelNames(List<NostrEvent> events) {
  final names = <String, String>{};
  for (final event in events) {
    if (event.kind != 39000) continue;
    final id = event.getTagValue('d');
    final name = event.getTagValue('name')?.trim();
    if (id == null || name == null || name.isEmpty) continue;
    final label = name.length > 100 ? name.substring(0, 100) : name;
    names[id] = label.startsWith('#') ? label : '#$label';
  }
  return names;
}
