import ActivityKit
import Foundation
import UIKit

struct AgentLiveActivityPayload: Equatable {
  let title: String
  let body: String
  let activeCount: Int
  let startedAtMillis: Int64
  let lastActivityAtMillis: Int64
  let expiresAtMillis: Int64
  let channelId: String
  let messageId: String?

  var contentState: AgentActivityAttributes.ContentState {
    AgentActivityAttributes.ContentState(
      title: title,
      body: body,
      activeAgentCount: activeCount,
      startedAtMillis: startedAtMillis,
      deepLink: deepLink
    )
  }

  var staleDate: Date {
    Date(timeIntervalSince1970: TimeInterval(expiresAtMillis) / 1_000)
  }

  var deepLink: String {
    guard let messageId else {
      return "buzz://"
    }
    var components = URLComponents()
    components.scheme = "buzz"
    components.host = "message"
    components.queryItems = [
      URLQueryItem(name: "channel", value: channelId),
      URLQueryItem(name: "id", value: messageId),
    ]
    return components.string ?? "buzz://"
  }

  static func from(arguments: Any?) -> AgentLiveActivityPayload? {
    guard let values = arguments as? [String: Any] else {
      return nil
    }
    guard
      let title = nonEmptyString(values["title"]),
      let body = nonEmptyString(values["body"]),
      let channelId = nonEmptyString(values["channelId"]),
      let activeCount = number(values["activeCount"])?.intValue,
      let startedAtMillis = number(values["startedAtMillis"])?.int64Value,
      let lastActivityAtMillis = number(values["lastActivityAtMillis"])?.int64Value,
      let expiresAtMillis = number(values["expiresAtMillis"])?.int64Value,
      activeCount > 0,
      startedAtMillis >= 0,
      lastActivityAtMillis >= startedAtMillis,
      expiresAtMillis > lastActivityAtMillis
    else {
      return nil
    }

    return AgentLiveActivityPayload(
      title: title,
      body: body,
      activeCount: activeCount,
      startedAtMillis: startedAtMillis,
      lastActivityAtMillis: lastActivityAtMillis,
      expiresAtMillis: expiresAtMillis,
      channelId: channelId,
      messageId: nonEmptyString(values["messageId"])
    )
  }

  private static func nonEmptyString(_ value: Any?) -> String? {
    guard let value = value as? String,
      !value.trimmingCharacters(in: .whitespaces).isEmpty
    else {
      return nil
    }
    return value
  }

  private static func number(_ value: Any?) -> NSNumber? {
    value as? NSNumber
  }
}

@available(iOS 16.1, *)
enum AgentLiveActivityManager {
  static func show(_ payload: AgentLiveActivityPayload) async throws -> Bool {
    guard ActivityAuthorizationInfo().areActivitiesEnabled else {
      return false
    }

    let activities = Activity<AgentActivityAttributes>.activities
    if let current = activities.first {
      await update(current, with: payload)
      for duplicate in activities.dropFirst() {
        await end(duplicate)
      }
      return true
    }

    let appIsActive = await MainActor.run {
      UIApplication.shared.applicationState == .active
    }
    guard appIsActive else {
      return false
    }

    let attributes = AgentActivityAttributes(source: "agent-activity")
    if #available(iOS 16.2, *) {
      let content = ActivityContent(
        state: payload.contentState,
        staleDate: payload.staleDate
      )
      _ = try Activity.request(
        attributes: attributes,
        content: content,
        pushType: nil
      )
    } else {
      _ = try Activity.request(
        attributes: attributes,
        contentState: payload.contentState,
        pushType: nil
      )
    }
    return true
  }

  static func dismiss() async {
    for activity in Activity<AgentActivityAttributes>.activities {
      await end(activity)
    }
  }

  private static func update(
    _ activity: Activity<AgentActivityAttributes>,
    with payload: AgentLiveActivityPayload
  ) async {
    if #available(iOS 16.2, *) {
      await activity.update(
        ActivityContent(
          state: payload.contentState,
          staleDate: payload.staleDate
        )
      )
    } else {
      await activity.update(using: payload.contentState)
    }
  }

  private static func end(
    _ activity: Activity<AgentActivityAttributes>
  ) async {
    if #available(iOS 16.2, *) {
      await activity.end(nil, dismissalPolicy: .immediate)
    } else {
      await activity.end(using: nil, dismissalPolicy: .immediate)
    }
  }
}
