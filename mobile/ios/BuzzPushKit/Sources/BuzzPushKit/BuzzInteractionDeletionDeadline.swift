import Foundation

/// Bounds an asynchronous communication-interaction deletion operation.
public final class BuzzInteractionDeletionDeadline {
  /// Callback-based interaction deletion operation.
  public typealias DeleteAllInteractions = (@escaping (Error?) -> Void) -> Void

  /// Schedules a timeout action after the supplied interval.
  public typealias ScheduleTimeout = (TimeInterval, @escaping () -> Void) -> Void

  private let deleteAllInteractions: DeleteAllInteractions
  private let scheduleTimeout: ScheduleTimeout
  private let timeout: TimeInterval

  /// Creates a deadline around an injected deletion operation and scheduler.
  public init(
    timeout: TimeInterval,
    deleteAllInteractions: @escaping DeleteAllInteractions,
    scheduleTimeout: @escaping ScheduleTimeout
  ) {
    self.timeout = timeout
    self.deleteAllInteractions = deleteAllInteractions
    self.scheduleTimeout = scheduleTimeout
  }

  /// Deletes all interactions, failing once if the callback misses its deadline.
  public func deleteAll(completion: @escaping (Error?) -> Void) {
    let completion = BuzzOneShotErrorCompletion(completion)
    scheduleTimeout(timeout) {
      completion.complete(
        NSError(
          domain: "BuzzInteractionDeletionDeadline",
          code: 1,
          userInfo: [
            NSLocalizedDescriptionKey: "Timed out deleting communication interactions."
          ]
        )
      )
    }
    deleteAllInteractions { error in
      completion.complete(error)
    }
  }
}

private final class BuzzOneShotErrorCompletion {
  private let lock = NSLock()
  private var completion: ((Error?) -> Void)?

  init(_ completion: @escaping (Error?) -> Void) {
    self.completion = completion
  }

  func complete(_ error: Error?) {
    lock.lock()
    let completion = completion
    self.completion = nil
    lock.unlock()
    completion?(error)
  }
}
