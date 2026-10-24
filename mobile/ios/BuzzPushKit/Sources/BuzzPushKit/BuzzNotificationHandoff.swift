import Foundation

/// Hands notification content back exactly once, before asynchronous cleanup.
public final class BuzzNotificationHandoff<Content> {
  private let lock = NSRecursiveLock()
  private var handler: ((Content) -> Void)?

  /// Creates a handoff shared by normal resolution and service expiration.
  public init(handler: @escaping (Content) -> Void) {
    self.handler = handler
  }

  /// Checks the native fence and synchronously delivers content or its safe fallback.
  /// `handoffIfAllowed` must invoke its closure synchronously when returning true.
  /// Cleanup runs only after restricted content has been handed off.
  public func finish(
    _ content: Content,
    restrictedFallback: Content,
    handoffIfAllowed: (() -> Void) -> Bool,
    cleanup: () -> Void
  ) {
    // Keep expiry from observing a consumed handler before delivery finishes.
    lock.lock()
    guard let handler else {
      lock.unlock()
      return
    }
    self.handler = nil
    let handedOff = handoffIfAllowed { handler(content) }
    if !handedOff { handler(restrictedFallback) }
    lock.unlock()
    if !handedOff { cleanup() }
  }
}
