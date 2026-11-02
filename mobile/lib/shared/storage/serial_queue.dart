/// Serializes operations without letting a failed operation poison later work.
class SerialQueue {
  Future<void>? _tail;

  Future<T> run<T>(Future<T> Function() operation) {
    final result = (_tail ?? Future<void>.value()).then((_) => operation());
    late final Future<void> tail;
    void release() {
      // An idle queue must not retain a completed future's scheduler/zone.
      // A later operation may already have replaced this tail.
      if (identical(_tail, tail)) _tail = null;
    }

    tail = result.then<void>(
      (_) => release(),
      onError: (Object _, StackTrace _) => release(),
    );
    _tail = tail;
    return result;
  }
}
