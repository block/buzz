import 'package:flutter/foundation.dart';

/// One item handed over by the iOS Share Extension.
///
/// Mirrors `BuzzShareInbox.Item` in `ios/BuzzShareInbox/BuzzShareInbox.swift`.
@immutable
class SharedItem {
  final SharedItemKind kind;

  /// Text body or URL string; null for files.
  final String? value;

  /// Absolute path of a file copied into the App Group inbox; null otherwise.
  final String? path;
  final String? name;
  final String? mimeType;

  const SharedItem({
    required this.kind,
    this.value,
    this.path,
    this.name,
    this.mimeType,
  });

  static SharedItem? fromMap(Object? raw) {
    if (raw is! Map) return null;
    final kind = switch (raw['kind']) {
      'text' => SharedItemKind.text,
      'url' => SharedItemKind.url,
      'file' => SharedItemKind.file,
      _ => null,
    };
    if (kind == null) return null;
    final value = raw['value'];
    final path = raw['path'];
    if (kind == SharedItemKind.file) {
      if (path is! String || path.isEmpty) return null;
    } else if (value is! String || value.trim().isEmpty) {
      return null;
    }
    return SharedItem(
      kind: kind,
      value: value is String ? value : null,
      path: path is String ? path : null,
      name: raw['name'] is String ? raw['name'] as String : null,
      mimeType: raw['mimeType'] is String ? raw['mimeType'] as String : null,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is SharedItem &&
      other.kind == kind &&
      other.value == value &&
      other.path == path &&
      other.name == name &&
      other.mimeType == mimeType;

  @override
  int get hashCode => Object.hash(kind, value, path, name, mimeType);
}

enum SharedItemKind { text, url, file }

/// Everything one share-sheet invocation delivered.
@immutable
class SharedPayload {
  final String id;
  final List<SharedItem> items;

  /// Destination picked inside the extension, when the sheet offered one.
  final String? communityId;
  final String? channelId;

  const SharedPayload({
    required this.id,
    required this.items,
    this.communityId,
    this.channelId,
  });

  bool get hasTarget =>
      communityId != null &&
      communityId!.isNotEmpty &&
      channelId != null &&
      channelId!.isNotEmpty;

  static SharedPayload? fromMap(Object? raw) {
    if (raw is! Map) return null;
    final id = raw['id'];
    final rawItems = raw['items'];
    if (id is! String || id.isEmpty || rawItems is! List) return null;
    final items = [for (final item in rawItems) ?SharedItem.fromMap(item)];
    if (items.isEmpty) return null;
    final communityId = raw['communityId'];
    final channelId = raw['channelId'];
    return SharedPayload(
      id: id,
      items: List.unmodifiable(items),
      communityId: communityId is String ? communityId : null,
      channelId: channelId is String ? channelId : null,
    );
  }

  /// Text and links joined into one composer body, or empty.
  String get composedText => [
    for (final item in items)
      if (item.kind != SharedItemKind.file) item.value!.trim(),
  ].join('\n');

  List<SharedItem> get files => [
    for (final item in items)
      if (item.kind == SharedItemKind.file) item,
  ];

  @override
  bool operator ==(Object other) =>
      other is SharedPayload &&
      other.id == id &&
      other.communityId == communityId &&
      other.channelId == channelId &&
      listEquals(other.items, items);

  @override
  int get hashCode =>
      Object.hash(id, communityId, channelId, Object.hashAll(items));
}
