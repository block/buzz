import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _appGroupChannel = MethodChannel('buzz/app_group');

/// The conversation identifier Buzz donates to iOS for share suggestions.
///
/// iOS hands it back to the share extension when the person picks that
/// suggestion. It carries the real device-local community ID and channel ID
/// and must match `BuzzPushPresentationIdentity.conversation` in BuzzPushKit.
String shareConversationIdentifier({
  required String communityId,
  required String channelId,
}) => jsonEncode([communityId, channelId]);

/// A conversation named by [shareConversationIdentifier].
typedef ShareTarget = ({String communityId, String channelId});

/// Items the share extension left for the app, and where they go.
class PendingShare {
  const PendingShare({
    required this.id,
    required this.target,
    required this.text,
    required this.files,
    this.missed = 0,
  });

  final String id;

  /// The suggested conversation the person picked, or null for the plain
  /// Buzz icon, which starts a new Solo chat.
  final ShareTarget? target;

  /// Shared URLs and text, in share-sheet order.
  final List<String> text;

  /// Paths of the shared images and videos copied into the app group.
  final List<String> files;

  /// Shared items the extension couldn't load or copy, so they are absent
  /// from [text] and [files].
  final int missed;

  /// The composer text the share adds: each URL or text item on its own line.
  String get composerText => text.join('\n');
}

/// Thrown when a pending share is missing or its manifest is malformed.
class PendingShareUnavailable implements Exception {
  const PendingShareUnavailable(this.message);
  final String message;
  @override
  String toString() => 'PendingShareUnavailable: $message';
}

Future<Directory> _sharesDirectory() async {
  final path = await _appGroupChannel.invokeMethod<String>(
    'pendingSharesDirectory',
  );
  if (path == null) {
    throw const PendingShareUnavailable('the app group is unavailable');
  }
  return Directory(path);
}

/// Reads the manifest the share extension wrote for [shareId].
Future<PendingShare> loadPendingShare(
  String shareId, {
  Future<Directory> Function() sharesDirectory = _sharesDirectory,
}) async {
  final directory = Directory('${(await sharesDirectory()).path}/$shareId');
  final manifest = File('${directory.path}/share.json');
  if (!await manifest.exists()) {
    throw PendingShareUnavailable('share $shareId has no manifest');
  }
  final json = jsonDecode(await manifest.readAsString());
  if (json is! Map || json['version'] != 1) {
    throw PendingShareUnavailable('share $shareId has an unknown manifest');
  }
  final text = json['text'];
  final files = json['files'];
  final target = json['target'];
  final missed = json['missed'] ?? 0;
  if (text is! List ||
      files is! List ||
      missed is! int ||
      (target != null && target is! String)) {
    throw PendingShareUnavailable('share $shareId has a malformed manifest');
  }
  return PendingShare(
    id: shareId,
    target: target == null ? null : parseShareConversationIdentifier(target),
    text: [for (final item in text) item as String],
    files: [for (final name in files) '${directory.path}/${_fileName(name)}'],
    missed: missed,
  );
}

String _fileName(Object? name) {
  if (name is! String || name.isEmpty || name.contains('/') || name == '..') {
    throw PendingShareUnavailable('invalid shared file name $name');
  }
  return name;
}

/// Parses an identifier iOS handed back. A target the app never donated
/// fails, so the caller reports that conversation as unavailable.
ShareTarget parseShareConversationIdentifier(String identifier) {
  final decoded = jsonDecode(identifier);
  if (decoded is List &&
      decoded.length == 2 &&
      decoded[0] is String &&
      decoded[1] is String) {
    return (communityId: decoded[0] as String, channelId: decoded[1] as String);
  }
  throw PendingShareUnavailable('unrecognized share target $identifier');
}

/// Deletes the share's copied items once the composer holds them.
Future<void> deletePendingShare(
  String shareId, {
  Future<Directory> Function() sharesDirectory = _sharesDirectory,
}) async {
  final directory = Directory('${(await sharesDirectory()).path}/$shareId');
  if (await directory.exists()) await directory.delete(recursive: true);
}

/// Shares waiting for a composer to open, keyed by compose draft key, oldest
/// first.
///
/// The dispatcher delivers a share here and then opens its conversation; that
/// conversation's composer takes each in turn and adds the items to its draft.
/// Shares queue rather than replace each other, since a conversation covered
/// by another page takes its shares only once it is back on screen.
class ComposerShareInbox extends Notifier<Map<String, List<PendingShare>>> {
  @override
  Map<String, List<PendingShare>> build() => const {};

  void deliver(String draftKey, PendingShare share) {
    final waiting = state[draftKey] ?? const <PendingShare>[];
    if (waiting.any((queued) => queued.id == share.id)) return;
    state = {
      ...state,
      draftKey: [...waiting, share],
    };
  }

  /// Removes and returns the oldest share waiting for [draftKey], if any.
  PendingShare? take(String draftKey) {
    final waiting = state[draftKey];
    if (waiting == null || waiting.isEmpty) return null;
    final rest = waiting.sublist(1);
    state = {...state}
      ..remove(draftKey)
      ..addAll({if (rest.isNotEmpty) draftKey: rest});
    return waiting.first;
  }
}

final composerShareInboxProvider =
    NotifierProvider<ComposerShareInbox, Map<String, List<PendingShare>>>(
      ComposerShareInbox.new,
    );
