import Foundation
import SwiftUI

/// Public relay metadata from its NIP-11 document (`application/nostr+json`),
/// the same unauthenticated endpoint the Dart community icon provider reads.
struct RelayInfo: Equatable {
  let name: String?
  let description: String?
  let icon: URL?
}

/// Drives the clip's single screen.
@MainActor
final class InviteClipModel: ObservableObject {
  enum Phase: Equatable {
    /// No invocation URL yet (e.g. launched from the Home Screen).
    case waitingForInvocation
    /// The URL is not an invite the app would accept.
    case unsupported(URL)
    /// A valid invite; metadata may still be loading or unavailable.
    case ready(BuzzInviteLink)
    /// The invite is stored for the full app.
    case saved(BuzzInviteLink)
    /// Storage failed; the user can retry or fall back to the web page.
    case failed(BuzzInviteLink, String)
  }

  @Published private(set) var phase: Phase = .waitingForInvocation
  @Published private(set) var relayInfo: RelayInfo?
  @Published private(set) var relayInfoFailed = false
  @Published var showAppStoreOverlay = false

  private let store: BuzzPendingInviteStore
  private let session: URLSession
  private var infoTask: Task<Void, Never>?
  private static let maximumInfoBytes = 512 * 1024

  init(store: BuzzPendingInviteStore, session: URLSession = .shared) {
    self.store = store
    self.session = session
  }

  var invite: BuzzInviteLink? {
    switch phase {
    case .ready(let invite), .saved(let invite), .failed(let invite, _): return invite
    case .waitingForInvocation, .unsupported: return nil
    }
  }

  /// Handle an invocation URL. Later invocations replace earlier ones.
  func receive(_ url: URL?) {
    guard let url else { return }
    infoTask?.cancel()
    relayInfo = nil
    relayInfoFailed = false
    guard let invite = BuzzInviteLink.parse(url) else {
      phase = .unsupported(url)
      return
    }
    phase = .ready(invite)
    infoTask = Task { await loadRelayInfo(for: invite) }
  }

  /// Park the invite for the full app, then recommend installing it.
  func join() {
    guard let invite else { return }
    do {
      try store.save(BuzzPendingInvite(url: invite.invocationURL.absoluteString, savedAt: Date()))
      phase = .saved(invite)
      showAppStoreOverlay = true
    } catch {
      phase = .failed(invite, error.localizedDescription)
    }
  }

  private func loadRelayInfo(for invite: BuzzInviteLink) async {
    guard let url = invite.relayInfoURL else {
      relayInfoFailed = true
      return
    }
    var request = URLRequest(url: url, timeoutInterval: 5)
    request.setValue("application/nostr+json", forHTTPHeaderField: "Accept")
    do {
      let (data, response) = try await session.data(for: request)
      guard !Task.isCancelled else { return }
      guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode),
        data.count <= Self.maximumInfoBytes, let document = try JSONSerialization.jsonObject(with: data) as? [String: Any]
      else {
        relayInfoFailed = true
        return
      }
      let iconValue = (document["icon"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
      let icon = URL(string: iconValue).flatMap { ["https", "http", "data"].contains($0.scheme ?? "") ? $0 : nil }
      relayInfo = RelayInfo(
        name: nonEmpty(document["name"] as? String),
        description: nonEmpty(document["description"] as? String),
        icon: icon
      )
    } catch {
      guard !Task.isCancelled else { return }
      relayInfoFailed = true
    }
  }

  private func nonEmpty(_ value: String?) -> String? {
    guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else { return nil }
    return trimmed
  }
}
