import Foundation

public struct PushCatchUpSelection: Sendable {
  public let event: VerifiedNostrEvent
  public let wasPreviouslyConsumed: Bool

  public init(event: VerifiedNostrEvent, wasPreviouslyConsumed: Bool) {
    self.event = event
    self.wasPreviouslyConsumed = wasPreviouslyConsumed
  }
}

public enum PushCatchUpStopReason: Equatable, Sendable {
  case complete
  case pageBudgetExceeded
  case deadlineExceeded
}

public struct PushCatchUpPager {
  /// The NSE has about eight seconds. Reserve two seconds for state persistence,
  /// duplicate absorption, and content delivery after bounded catch-up.
  public static let traversalSeconds: TimeInterval = 6
  public static let pageLimit = 10
  public static let maximumPages = 12

  private struct Query {
    let filter: PushLeaseFilter
    let hTag: String?
  }

  private let queries: [Query]
  private let since: Int?
  private let deadline: Date
  private let pageLimit: Int
  private let maximumPages: Int
  private var subscriptionIndex: Int
  private var rawTail: PushEventPosition?
  private var pagesRequested = 0

  public private(set) var stopReason: PushCatchUpStopReason?

  public init(
    subscriptions: [PushLeaseSubscription],
    since: Int?,
    scan: PushCatchUpScan = PushCatchUpScan(),
    startedAt: Date = Date(),
    traversalSeconds: TimeInterval = Self.traversalSeconds,
    pageLimit: Int = Self.pageLimit,
    maximumPages: Int = Self.maximumPages
  ) {
    precondition(traversalSeconds > 0, "Push catch-up traversal allowance must be positive")
    precondition(pageLimit > 0, "Push catch-up page limit must be positive")
    precondition(maximumPages > 0, "Push catch-up page budget must be positive")
    queries = subscriptions.flatMap { subscription in
      guard let hTags = subscription.filter.hTags, hTags.count > 1 else {
        return [Query(filter: subscription.filter, hTag: nil)]
      }
      return hTags.map { Query(filter: subscription.filter, hTag: $0) }
    }
    self.since = since
    deadline = startedAt.addingTimeInterval(traversalSeconds)
    self.pageLimit = pageLimit
    self.maximumPages = maximumPages

    // A subscription snapshot can legitimately shrink between NSE wakes. Treat
    // a scan beyond the new query set as exhausted so the caller clears it and
    // begins a fresh pass on the next wake.
    if scan.subscriptionIndex >= queries.count {
      subscriptionIndex = queries.count
      rawTail = nil
    } else {
      subscriptionIndex = scan.subscriptionIndex
      rawTail = scan.before
    }
  }

  /// The position to persist for the next wake. A completed traversal clears
  /// its scan so newly arrived events are visible from the top of the next pass.
  public var scan: PushCatchUpScan {
    guard stopReason != .complete else { return PushCatchUpScan() }
    return PushCatchUpScan(subscriptionIndex: subscriptionIndex, before: rawTail)
  }

  public func remainingTraversalSeconds(now: Date = Date()) -> TimeInterval {
    max(0, deadline.timeIntervalSince(now))
  }

  /// Return the next raw relay page. Continuation is derived only from the
  /// observed raw page tail, never from post-selection or delivered-ID state.
  public mutating func nextFilter(now: Date = Date()) -> [String: Any]? {
    guard stopReason == nil else { return nil }
    guard subscriptionIndex < queries.count else {
      stopReason = .complete
      return nil
    }
    guard pagesRequested < maximumPages else {
      stopReason = .pageBudgetExceeded
      return nil
    }
    guard now < deadline else {
      stopReason = .deadlineExceeded
      return nil
    }

    let query = queries[subscriptionIndex]
    var filter = query.filter.queryFilter(since: since, limit: pageLimit)
    if let hTag = query.hTag {
      // The HTTP relay narrows a multi-value #h filter to its first channel.
      // One filter per channel avoids depending on that broken contract.
      filter["#h"] = [hTag]
    }
    if let rawTail {
      filter["until"] = rawTail.createdAt
      filter["before_id"] = rawTail.id
    }
    pagesRequested += 1
    return filter
  }

  public mutating func receive(rawPage: [VerifiedNostrEvent]) {
    guard stopReason == nil else { return }
    let ordered = rawPage.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
    }
    if ordered.count < pageLimit {
      // `/query` exposes no exhaustion signal, so a short page is treated as
      // exhausted even though post-LIMIT rejection makes that inference unsound.
      // Relay support for an explicit exhaustion signal is required to fix this.
      subscriptionIndex += 1
      rawTail = nil
      if subscriptionIndex == queries.count {
        stopReason = .complete
      }
    } else {
      rawTail = ordered.last.map {
        PushEventPosition(createdAt: $0.createdAt, id: $0.id)
      }
    }
  }
}

public enum PushCatchUp {
  /// Return selectable events first and consumed duplicate fallbacks last.
  /// Duplicate cleanup therefore never competes with forward progress.
  public static func orderedSelections(
    events: [VerifiedNostrEvent],
    origin: String,
    subscriptions: [PushLeaseSubscription],
    consumptionState: PushConsumptionState,
    verify: (VerifiedNostrEvent) -> Bool = { $0.hasValidIDAndSignature() }
  ) -> [PushCatchUpSelection] {
    var eventsByID: [String: VerifiedNostrEvent] = [:]
    for event in events where verify(event) {
      guard subscriptions.contains(where: {
        PushLeaseMatcher.matches(event: event, subscription: $0)
      }) else {
        continue
      }
      eventsByID[event.id] = event
    }

    let ordered = eventsByID.values.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt < $1.createdAt
    }
    let selectable = ordered.compactMap { event -> PushCatchUpSelection? in
      let position = PushEventPosition(createdAt: event.createdAt, id: event.id)
      guard consumptionState.canSelect(position, for: origin) else { return nil }
      return PushCatchUpSelection(event: event, wasPreviouslyConsumed: false)
    }
    let duplicateFallbackID = consumptionState.state(for: origin).lastDisplayed?.id
    let duplicates = ordered.compactMap { event -> PushCatchUpSelection? in
      guard event.id == duplicateFallbackID,
        consumptionState.hasConsumed(eventID: event.id, for: origin)
      else {
        return nil
      }
      return PushCatchUpSelection(event: event, wasPreviouslyConsumed: true)
    }
    return selectable + duplicates
  }
}
