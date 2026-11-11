import 'dart:convert';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/relay/relay.dart';
import '../../shared/crypto/event_signing.dart';
import 'channel_window.dart';

/// The complete request scope authenticated by a NIP-CW thread bound.
class ThreadWindowRequest {
  final String relayUrl;
  final String reader;
  final String relayAuthor;
  final NostrFilter filter;

  const ThreadWindowRequest({
    required this.relayUrl,
    required this.reader,
    required this.relayAuthor,
    required this.filter,
  });

  /// Canonical request binding from NIP-CW, including the host and reader.
  String get boundsKey {
    var host = Uri.parse(relayUrl).authority.toLowerCase();
    for (final suffix in [':443', ':80']) {
      if (host.endsWith(suffix)) {
        host = host.substring(0, host.length - suffix.length);
        break;
      }
    }
    if (host.endsWith('.')) host = host.substring(0, host.length - 1);
    final kinds = filter.kinds.toSet().toList()..sort();
    final binding = jsonEncode([
      'tw',
      1,
      'older',
      host,
      reader.toLowerCase(),
      filter.tags['#h']!.single.toLowerCase(),
      filter.tags['#e']!.single.toLowerCase(),
      filter.limit,
      filter.extensions['depth_limit'],
      kinds,
      if (filter.until == null)
        null
      else
        [filter.until, filter.extensions['before_id']],
      filter.extensions['include_aux'] == true,
    ]);
    return 'tw:1:${sha256.convert(utf8.encode(binding))}';
  }
}

/// Verifies bounds and parses rows off the UI isolate. Failed pages never
/// advance a cursor and never fall back to an unrelated query mode.
Future<ChannelWindowPage> decodeThreadWindow(
  List<NostrEvent> events,
  ThreadWindowRequest request,
  ChannelPageCursor? cursor,
) => compute(_decodeThreadWindow, (events, request, cursor));

ChannelWindowPage _decodeThreadWindow(
  (List<NostrEvent>, ThreadWindowRequest, ChannelPageCursor?) input,
) {
  final (events, request, cursor) = input;
  final channel = request.filter.tags['#h']!.single;
  final root = request.filter.tags['#e']!.single;
  final bounds = events.where((e) => e.kind == EventKind.threadWindowBounds);
  if (bounds.length != 1) {
    throw const FormatException('Thread window is unavailable or incomplete');
  }
  final bound = bounds.single;
  final expectedTags = {'d': request.boundsKey, 'h': channel, 'e': root};
  if (bound.pubkey != request.relayAuthor ||
      bound.tags.length != 3 ||
      expectedTags.entries.any(
        (entry) =>
            bound.tags
                .where(
                  (tag) =>
                      tag.length == 2 &&
                      tag[0] == entry.key &&
                      tag[1] == entry.value,
                )
                .length !=
            1,
      )) {
    throw const FormatException('Thread bounds do not match the request');
  }
  final signed = nostr.Event.fromMap(bound.toJson(), verify: false);
  if (signed.getEventId() != bound.id ||
      !verifySchnorrHex(
        publicKey: bound.pubkey,
        messageHex: bound.id,
        signature: bound.sig,
      )) {
    throw const FormatException('Invalid thread bounds signature');
  }
  final content = jsonDecode(bound.content);
  if (content is! Map<String, dynamic> ||
      content['version'] != 1 ||
      content['direction'] != 'older' ||
      content['has_more'] is! bool) {
    throw const FormatException('Invalid thread bounds content');
  }
  final next = parseWindowCursor(content['next_cursor']);
  final hasMore = content['has_more'] as bool;
  if (hasMore != (next != null) ||
      (next != null && cursor != null && !isChannelCursorOlder(next, cursor))) {
    throw const FormatException('Invalid thread continuation');
  }
  final rows = <ChannelWindowRow>[];
  final aux = <NostrEvent>[];
  for (final event in events) {
    if (request.filter.kinds.contains(event.kind)) {
      // Bounds authenticate pagination, not individual row ancestry. The
      // forum resolver validates the complete parent path separately.
      if (event.channelId != channel || event.id == root) {
        throw const FormatException('Invalid thread row scope');
      }
      rows.add(ChannelWindowRow(event: event));
    } else if (EventKind.channelAuxEventKinds.contains(event.kind)) {
      aux.add(event);
    } else if (event.kind != EventKind.threadWindowBounds) {
      throw const FormatException('Unexpected thread window event');
    }
  }
  if (rows.length > request.filter.limit) {
    throw const FormatException('Thread window exceeds row limit');
  }
  // Reuse channel-window order, overlap and cursor-chain validation.
  final page = ChannelWindowPage(
    startCursor: cursor,
    rows: rows,
    aux: aux,
    nextCursor: next,
    hasMore: hasMore,
  );
  if (cursor == null) {
    replaceNewestChannelWindow(const ChannelWindowStore.empty(), page);
  }
  return page;
}

/// Strict composite cursor grammar shared by forum window readers.
ChannelPageCursor? parseWindowCursor(Object? value) {
  if (value == null) return null;
  if (value is! Map<String, dynamic> ||
      value['created_at'] is! int ||
      (value['created_at'] as int) < 0 ||
      value['id'] is! String ||
      !RegExp(r'^[0-9a-f]{64}$').hasMatch(value['id'] as String)) {
    throw const FormatException('Invalid window cursor');
  }
  return ChannelPageCursor(
    createdAt: value['created_at'] as int,
    eventId: value['id'] as String,
  );
}
