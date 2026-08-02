import Foundation

public enum PushWatermark {
    /// Nostr permits modest clock drift, but an authenticated author must not
    /// be able to pin notification queries arbitrarily far into the future.
    public static let allowedFutureSkewSeconds = 300
    public static let keyPrefix = "buzz.push.watermark."

    public static func key(communityID: String) -> String {
        keyPrefix + communityID
    }

    public static func persistedTimestamp(
        eventTimestamp: Int,
        now: Int = Int(Date().timeIntervalSince1970),
        allowedFutureSkew: Int = allowedFutureSkewSeconds
    ) -> Int {
        min(eventTimestamp, now + allowedFutureSkew)
    }

    public static func isAcceptable(
        eventTimestamp: Int,
        now: Int = Int(Date().timeIntervalSince1970),
        allowedFutureSkew: Int = allowedFutureSkewSeconds
    ) -> Bool {
        eventTimestamp <= now + allowedFutureSkew
    }

    public static func queryTimestamp(
        storedWatermark: Int,
        now: Int = Int(Date().timeIntervalSince1970)
    ) -> Int {
        min(storedWatermark, now)
    }

    /// Nostr `since` is inclusive. Keeping the watermark itself allows a later
    /// event created in the same second to remain queryable.
    public static func querySince(watermark: Int) -> Int? {
        watermark > 0 ? watermark : nil
    }

    public static func staleKeys(
        in storedKeys: [String],
        activeCommunityIDs: Set<String>
    ) -> [String] {
        storedKeys.filter { key in
            guard key.hasPrefix(keyPrefix) else { return false }
            return !activeCommunityIDs.contains(String(key.dropFirst(keyPrefix.count)))
        }
    }
}
