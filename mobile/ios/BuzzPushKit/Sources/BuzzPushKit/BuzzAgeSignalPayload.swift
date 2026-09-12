import Foundation

/// Converts Apple's age gates to the inclusive upper age used by Flutter.
public enum BuzzAgeSignalPayload {
  /// Apple's upper bound is the gate the person is under; nil means unbounded.
  public static func sharing(exclusiveUpperBound: Int?) -> [String: Any] {
    let ageUpper = exclusiveUpperBound.map { ($0 - 1) as Any } ?? NSNull()
    return ["status": "signal", "ageUpper": ageUpper]
  }
}
