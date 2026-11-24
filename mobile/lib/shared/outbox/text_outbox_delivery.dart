import 'dart:async';

import 'package:flutter/foundation.dart';

import '../mentions/mention_tags.dart';
import '../relay/relay.dart';
import 'text_outbox_store.dart';

/// Delivers a scope's queued messages sequentially. The relay adapter checks
/// [beforeWire] after backpressure and before putting an event on the socket.
final class TextOutboxDelivery {
  TextOutboxDelivery({
    required this.store,
    required this.prepare,
    required this.releaseDraft,
    required this.publish,
    required this.lookup,
    required this.showAccepted,
    this.showNotAccepted,
    DateTime Function()? now,
  }) : _now = now ?? DateTime.now;

  final TextOutboxStore store;
  final Future<bool> Function(TextOutboxMessage) releaseDraft;
  final Future<NostrEvent> Function(TextOutboxMessage, int Function()) prepare;
  final Future<void> Function(
    NostrEvent, {
    required Future<bool> Function() beforeWire,
    required void Function() validateScope,
  })
  publish;
  final Future<bool> Function(NostrEvent) lookup;
  final void Function(TextOutboxMessage, NostrEvent) showAccepted;
  final void Function(TextOutboxMessage, NostrEvent)? showNotAccepted;
  final DateTime Function() _now;

  /// Delivers pending messages in commit order. Returns true when a temporary
  /// failure needs a scheduled retry.
  Future<bool> drain(TextOutboxScope scope, bool Function() isCurrent) async {
    for (final message in await store.list(scope)) {
      if (!isCurrent()) return false;
      if (message.status != TextOutboxStatus.pending) continue;
      final retry = await _send(scope, message, isCurrent);
      if (retry) return true;
    }
    return false;
  }

  bool _expired(NostrEvent event) =>
      (_now().millisecondsSinceEpoch ~/ 1000 - event.createdAt).abs() > 900;

  Future<bool> _send(
    TextOutboxScope scope,
    TextOutboxMessage message,
    bool Function() isCurrent,
  ) async {
    void validateScope() {
      if (!isCurrent() || store.isRetiring(scope.communityId)) {
        throw ActiveCommunityChangedError('Queued send changed communities.');
      }
    }

    Future<void> accept(NostrEvent event) async {
      validateScope();
      showAccepted(message, event);
      await store.remove(scope, message.id);
    }

    Future<void> reconcile(NostrEvent event) async {
      var found = false;
      try {
        found = await lookup(event);
      } catch (error) {
        debugPrint('Outbox confirmation read failed: ${error.runtimeType}');
      }
      validateScope();
      if (found) {
        await accept(event);
      } else {
        await store.setStatus(scope, message.id, TextOutboxStatus.unconfirmed);
        if (isCurrent()) showNotAccepted?.call(message, event);
      }
    }

    var event = message.signedEvent;
    var attempted = message.mayHaveSent;
    try {
      if (!await releaseDraft(message)) return true;
      validateScope();
      if (event != null && _expired(event) && attempted) {
        await reconcile(event);
        return false;
      }
      if (event == null || _expired(event)) {
        event = await prepare(message, () => _now().millisecondsSinceEpoch);
        validateScope();
        if (event.pubkey != scope.pubkey) {
          throw ActiveCommunityChangedError('Queued send changed signers.');
        }
        await store.saveSignature(scope, message.id, event);
      }
      final signed = event;
      validateScope();
      await publish(
        signed,
        beforeWire: () async {
          validateScope();
          if (_expired(signed)) throw const _OutboxSignatureExpired();
          await store.markAttempted(scope, message.id);
          attempted = true;
          return isCurrent();
        },
        validateScope: validateScope,
      );
      await accept(signed);
      return false;
    } on ActiveCommunityChangedError {
      return false;
    } on _OutboxSignatureExpired {
      if (attempted && event != null) await reconcile(event);
      return !attempted && isCurrent();
    } on RelayEventRejectedException catch (error) {
      if (!isCurrent()) return false;
      final prefix = error.message.split(':').first;
      final transient = prefix == 'rate-limited' || prefix == 'error';
      final definitive = const {
        'invalid',
        'restricted',
        'blocked',
        'auth-required',
        'rate-limited',
      }.contains(prefix);
      await store.recordRejection(
        scope,
        message.id,
        status: transient ? TextOutboxStatus.pending : TextOutboxStatus.failed,
        definitive: definitive,
      );
      if (!transient && event != null && isCurrent()) {
        showNotAccepted?.call(message, event);
      }
      return transient;
    } on MentionTagException {
      if (!isCurrent()) return false;
      await store.setStatus(scope, message.id, TextOutboxStatus.failed);
      return false;
    } catch (error) {
      debugPrint('Queued send will retry: ${error.runtimeType}');
      return isCurrent();
    }
  }
}

final class _OutboxSignatureExpired implements Exception {
  const _OutboxSignatureExpired();
}
