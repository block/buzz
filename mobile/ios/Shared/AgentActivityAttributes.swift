import ActivityKit
import Foundation

struct AgentActivityAttributes: ActivityAttributes {
  struct ContentState: Codable, Hashable {
    let title: String
    let body: String
    let activeAgentCount: Int
    let startedAtMillis: Int64
    let deepLink: String

    var publicTitle: String {
      activeAgentCount == 1
        ? "Agent is working"
        : "\(activeAgentCount) agents are working"
    }

    var publicBody: String {
      "Open Buzz to view progress"
    }

    var startedAt: Date {
      Date(timeIntervalSince1970: TimeInterval(startedAtMillis) / 1_000)
    }
  }

  let source: String
}
