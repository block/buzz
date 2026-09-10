import '../crypto/signed_event.dart';
import 'nostr_models.dart';

typedef _Coordinate = (int, String, String?);
typedef _Order = (int, String);

bool _newer(_Order a, _Order b) =>
    a.$1 > b.$1 || (a.$1 == b.$1 && a.$2.compareTo(b.$2) < 0);

class _EvidenceCell {
  _Order? latest;
  bool retired = false;
}

/// Bounded session-local observation order, not an authorization/profile cache.
/// Only signed events observed on this session can invalidate a query snapshot.
/// Eviction retires just the affected coordinate's outstanding capabilities.
class RelayEvidenceClock {
  final _cells = <_Coordinate, _EvidenceCell>{};
  static const _capacity = 8192;

  _EvidenceCell _cell(_Coordinate key) {
    final cell = _cells.remove(key) ?? _EvidenceCell();
    _cells[key] = cell;
    while (_cells.length > _capacity) {
      _cells.remove(_cells.keys.first)!.retired = true;
    }
    return cell;
  }

  /// Retire outstanding capabilities on session rebuild/identity changes.
  void clear() {
    for (final cell in _cells.values) {
      cell.retired = true;
    }
    _cells.clear();
  }

  /// Observe immediately on socket receipt, before UI batching/debouncing.
  void observe(NostrEvent event) {
    if (!const [0, 10100, 30177, 39002].contains(event.kind) ||
        !verifySignedEvent(event)) {
      return;
    }
    final identifiers = event.kind >= 30000
        ? event.tags
              .where((t) => t.length >= 2 && t[0] == 'd')
              .map((t) => t[1])
              .toSet()
        : <String?>{null};
    for (final identifier in identifiers) {
      final cell = _cell((event.kind, event.pubkey, identifier));
      final order = (event.createdAt, event.id);
      if (cell.latest == null || _newer(order, cell.latest!)) {
        cell.latest = order;
      }
    }
  }

  /// Retain a coordinate across an in-flight read. Capacity eviction fails
  /// that read closed instead of making lost evidence look absent again.
  bool Function() retain(int kind, String author, String? identifier) {
    final cell = _cell((kind, author, identifier));
    return () => !cell.retired;
  }

  /// Fence an exact query's newest head, including negative/absent evidence.
  /// Unrelated authors/coordinates never invalidate this capability. Capturing
  /// after query completion still compares against events observed during it.
  bool Function() snapshot(
    int kind,
    String author,
    String? identifier,
    Iterable<NostrEvent> events,
  ) {
    final cell = _cell((kind, author, identifier));
    _Order? head;
    for (final event in events) {
      if (event.kind != kind ||
          event.pubkey != author ||
          (kind >= 30000 && event.getTagValue('d') != identifier)) {
        continue;
      }
      final order = (event.createdAt, event.id);
      if (head == null || _newer(order, head)) head = order;
    }
    return () =>
        !cell.retired &&
        (cell.latest == null || (head != null && !_newer(cell.latest!, head)));
  }
}
