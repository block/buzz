import Foundation

/// Prevents cleanup from checkpointing work produced for an obsolete APNs token.
public enum BuzzPushCleanupTokenFence {
  /// Runs `checkpoint` only when a nonempty captured token is still current.
  @discardableResult
  public static func checkpointIfCurrent(
    capturedDeviceToken: Data?,
    liveDeviceToken: Data?,
    checkpoint: () throws -> Void
  ) rethrows -> Bool {
    guard let capturedDeviceToken, !capturedDeviceToken.isEmpty,
      capturedDeviceToken == liveDeviceToken
    else {
      return false
    }
    try checkpoint()
    return true
  }
}
