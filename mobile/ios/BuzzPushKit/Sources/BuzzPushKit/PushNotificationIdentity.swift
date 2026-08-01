import Foundation

public enum PushNotificationIdentityError: Error, Equatable {
  case missingIdentity
  case malformedIdentity
}

public struct PushNotificationIdentity: Codable, Equatable, Sendable {
  public static let userInfoKey = "buzz_push_identity"

  public let eventID: String
  public let origin: String

  enum CodingKeys: String, CodingKey {
    case eventID = "event_id"
    case origin
  }

  public init(eventID: String, origin: String) {
    self.eventID = eventID.lowercased()
    self.origin = origin
  }

  public var userInfoValue: [String: String] {
    ["event_id": eventID, "origin": origin]
  }

  public static func decodeIfPresent(
    from userInfo: [AnyHashable: Any]
  ) throws -> PushNotificationIdentity? {
    guard let raw = userInfo[userInfoKey] else { return nil }
    guard let fields = raw as? [String: String],
      let eventID = fields["event_id"],
      let origin = fields["origin"],
      fields.count == 2,
      !eventID.isEmpty,
      !origin.isEmpty
    else {
      throw PushNotificationIdentityError.malformedIdentity
    }
    return PushNotificationIdentity(eventID: eventID, origin: origin)
  }

  public static func require(
    from userInfo: [AnyHashable: Any]
  ) throws -> PushNotificationIdentity {
    guard let identity = try decodeIfPresent(from: userInfo) else {
      throw PushNotificationIdentityError.missingIdentity
    }
    return identity
  }
}

public struct PushDeliveredNotificationRecord {
  public let requestIdentifier: String
  public let userInfo: [AnyHashable: Any]

  public init(requestIdentifier: String, userInfo: [AnyHashable: Any]) {
    self.requestIdentifier = requestIdentifier
    self.userInfo = userInfo
  }
}

public enum PushDuplicateAbsorption {
  public static func requestIdentifiersToRemove(
    matching identity: PushNotificationIdentity,
    from delivered: [PushDeliveredNotificationRecord]
  ) throws -> [String] {
    try delivered.compactMap { notification in
      guard let deliveredIdentity = try PushNotificationIdentity.decodeIfPresent(
        from: notification.userInfo
      ) else {
        return nil
      }
      return deliveredIdentity == identity ? notification.requestIdentifier : nil
    }
  }
}
