import AuthenticationServices
import Flutter
import UIKit

/// Keeps the authentication callback scoped to the system browser session.
final class BuzzBrowserAuthBridge: NSObject, ASWebAuthenticationPresentationContextProviding {
  private let channel: FlutterMethodChannel
  private var session: ASWebAuthenticationSession?
  private var pending: FlutterResult?
  private var attemptID: UUID?

  private let isActive: () -> Bool
  private let startSession: (ASWebAuthenticationSession) -> Bool
  private var activationObserver: NSObjectProtocol?
  private var activationTimeout: DispatchWorkItem?

  init(
    messenger: FlutterBinaryMessenger,
    isActive: @escaping () -> Bool = { UIApplication.shared.applicationState == .active },
    startSession: @escaping (ASWebAuthenticationSession) -> Bool = { $0.start() }
  ) {
    self.isActive = isActive
    self.startSession = startSession
    channel = FlutterMethodChannel(name: "buzz/browser_auth", binaryMessenger: messenger)
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return }
      if call.method == "cancel" {
        self.session?.cancel()
        self.finish(nil, code: "cancelled")
        result(nil)
        return
      }
      guard call.method == "start" else { result(FlutterMethodNotImplemented); return }
      guard self.session == nil,
        let args = call.arguments as? [String: String],
        let raw = args["url"], let url = URL(string: raw),
        url.scheme == "https",
        ["app.builderlab.xyz", "app.builderlabstaging.xyz"].contains(url.host ?? ""),
        ["/api/goose/v1/auth/login", "/auth/native"].contains(url.path),
        url.user == nil, url.password == nil,
        url.port == nil, url.fragment == nil
      else { result(FlutterError(code: "unavailable", message: nil, details: nil)); return }
      self.pending = result
      let attemptID = UUID()
      self.attemptID = attemptID
      let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "xyz.block.buzz.mobile") { [weak self] callback, error in
        DispatchQueue.main.async {
          guard self?.attemptID == attemptID else { return }
          let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
          self?.finish(callback?.absoluteString, code: cancelled ? "cancelled" : "unavailable")
        }
      }
      session.presentationContextProvider = self
      session.prefersEphemeralWebBrowserSession = false
      #if DEBUG
      session.prefersEphemeralWebBrowserSession = args["ephemeral"] == "true"
      #endif
      self.session = session
      self.startWhenActive(attemptID)

    }
  }

  // Safari delivers a deep link before UIKit finishes activating our app.
  // ASWebAuthenticationSession.start() rejects presentation during that handoff.
  private func startWhenActive(_ id: UUID) {
    guard attemptID == id, let session else { return }
    if isActive() {
      clearActivationWait()
      if !startSession(session) { finish(nil, code: "unavailable") }
      return
    }
    activationObserver = NotificationCenter.default.addObserver(
      forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { [weak self] _ in
      guard let self, self.attemptID == id, self.isActive() else { return }
      self.startWhenActive(id)
    }
    let timeout = DispatchWorkItem { [weak self] in
      guard let self, self.attemptID == id else { return }
      self.finish(nil, code: "unavailable")
    }
    activationTimeout = timeout
    DispatchQueue.main.asyncAfter(deadline: .now() + 10, execute: timeout)
  }

  private func clearActivationWait() {
    if let activationObserver { NotificationCenter.default.removeObserver(activationObserver) }
    activationObserver = nil
    activationTimeout?.cancel()
    activationTimeout = nil
  }

  deinit { clearActivationWait() }

  private func finish(_ callback: String?, code: String) {
    clearActivationWait()
    let result = pending
    attemptID = nil
    pending = nil
    session = nil
    if let callback { result?(callback) }
    else { result?(FlutterError(code: code, message: nil, details: nil)) }
  }

  func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
      .flatMap { $0.windows }.first(where: { $0.isKeyWindow }) ?? ASPresentationAnchor()
  }
}
