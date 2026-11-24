import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../shared/relay/nostr_models.dart';

/// Carries submitted composer geometry through asynchronous send preparation.
class ComposerSendOrigin {
  static final _zoneKey = Object();

  static final _insertionKey = Object();

  static final _connectionFallbackKey = Object();

  /// Clears the submitted draft when its optimistic row enters the timeline.
  /// The optional fallback saves text when that send loses its connection.
  static Future<void> uponLocalInsertion(
    VoidCallback inserted,
    Future<void> Function() send, {
    Future<void> Function(NostrEvent?)? connectionFallback,
  }) async {
    try {
      await runZoned(
        send,
        zoneValues: {
          _insertionKey: inserted,
          _connectionFallbackKey: connectionFallback,
        },
      );
    } on _ComposerSendQueued {
      // The draft is now owned by the outbox. Relay acceptance has its own
      // callbacks when the runner delivers the saved event.
    }
  }

  /// Ends a composer submission whose text was saved for later delivery.
  static Never finishQueuedSend() => throw const _ComposerSendQueued();

  /// Saves the exact event if it may have reached the relay, or unsigned text
  /// if the relay session proves it never wrote the event.
  static Future<void>? queueAfterConnectionLoss(NostrEvent? possiblySent) =>
      (Zone.current[_connectionFallbackKey]
              as Future<void> Function(NostrEvent?)?)
          ?.call(possiblySent);

  static void didInsertLocalMessage() =>
      (Zone.current[_insertionKey] as VoidCallback?)?.call();

  /// Captures only geometry; no draft or identity data is retained.
  static Rect? capture(GlobalKey key) {
    final box = key.currentContext?.findRenderObject();
    return box is RenderBox && box.hasSize && box.attached
        ? box.localToGlobal(Offset.zero) & box.size
        : null;
  }

  /// Geometry for this send, isolated from other concurrent submissions.
  static Rect? get current => Zone.current[_zoneKey] as Rect?;

  /// Retains the submitted surface until this send creates its local claim.
  static Future<void> run(Rect? origin, Future<void> Function() send) =>
      runZoned(send, zoneValues: {_zoneKey: origin});
}

final class _ComposerSendQueued implements Exception {
  const _ComposerSendQueued();
}
