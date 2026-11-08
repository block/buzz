import Foundation

/// Converts Apple's age gates to the inclusive upper age used by Flutter.
public enum BuzzAgeSignalPayload {
  /// Apple's upper bound is the gate the person is under; nil means unbounded.
  public static func sharing(exclusiveUpperBound: Int?, lowerBound: Int? = nil) -> [String: Any] {
    // Buzz requests only the 18-year gate. Unexpected bounds are not evidence
    // of minority; never subtract an unvalidated integer (including Int.min).
    let validLower = lowerBound.map { $0 >= 0 && $0 < 18 } ?? true
    let ageUpper: Any = exclusiveUpperBound == 18 && validLower ? 17 : NSNull()
    return ["status": "signal", "ageUpper": ageUpper]
  }
}
