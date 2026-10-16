import Foundation

/// A relay invite parsed from an invocation URL.
///
/// Swift mirror of `parseInviteDeepLink` in
/// `lib/shared/deeplink/deep_link.dart`. The App Clip only needs to decide
/// whether a scanned URL is an invite it may carry across the install and
/// what to show for it; the full app re-parses the stored URL with the Dart
/// parser, so this type must never accept a URL that Dart would reject.
struct BuzzInviteLink: Equatable {
  /// Relay origin normalized to the websocket scheme the app connects with.
  let relayURL: String

  /// Invite code from the link.
  let code: String

  /// Optional receipt proving acceptance of the relay's current join policy.
  let policyReceipt: String?

  /// The URL exactly as it was received; this is what the full app replays.
  let invocationURL: URL

  /// `host[:port]`, matching the host line of the Dart invite sheet.
  var host: String {
    guard let components = URLComponents(string: relayURL), let host = components.host else {
      return relayURL
    }
    if let port = components.port { return "\(host):\(port)" }
    return host
  }

  /// Mirrors `Community.nameFromUrl`: the fallback name before NIP-11 answers.
  var communityName: String {
    guard let host = URLComponents(string: relayURL)?.host, !host.isEmpty else { return "Community" }
    if host.contains("localhost") || host == "127.0.0.1" { return "Local Dev" }
    let parts = host.split(separator: ".", omittingEmptySubsequences: false)
    return parts.count > 2 ? String(parts[0]) : host
  }

  /// Public NIP-11 document URL for the relay, as the Dart icon provider derives it.
  var relayInfoURL: URL? {
    guard var components = URLComponents(string: relayURL) else { return nil }
    components.scheme = components.scheme == "wss" ? "https" : "http"
    components.path = "/"
    return components.url
  }

  /// Parse canonical HTTPS invite links and `buzz://join` handoffs.
  ///
  /// `allowInsecureLocalhost` mirrors Dart's `kDebugMode` default: plaintext
  /// `http://localhost` invites resolve to `ws://localhost` in debug builds only.
  static func parse(_ url: URL, allowInsecureLocalhost: Bool = isDebugBuild) -> BuzzInviteLink? {
    guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
    if components.fragment != nil || hasUserInfo(components) { return nil }

    switch components.scheme?.lowercased() {
    case "buzz":
      guard components.host?.lowercased() == "join" else { return nil }
      let query = singleValuedQuery(components)
      guard let relay = query["relay"], !relay.isEmpty, let code = query["code"], !code.isEmpty,
        let relayComponents = URLComponents(string: relay), let relayScheme = relayComponents.scheme?.lowercased(),
        relayScheme == "ws" || relayScheme == "wss", let relayHost = relayComponents.host, !relayHost.isEmpty,
        !hasUserInfo(relayComponents), relayComponents.fragment == nil,
        BuzzRelayOriginRules.isAcceptable(relayComponents, allowInsecureLocalhost: allowInsecureLocalhost)
      else { return nil }
      let receipt = query["policy_receipt"]
      return BuzzInviteLink(
        relayURL: origin(scheme: relayScheme, host: relayHost, port: relayComponents.port),
        code: code,
        policyReceipt: receipt?.isEmpty == false ? receipt : nil,
        invocationURL: url
      )
    case "https", "http":
      guard let host = components.host, !host.isEmpty else { return nil }
      // Dart's `pathSegments` keeps a trailing empty segment, so `/invite/x/`
      // is three segments and rejected; drop only the leading slash here.
      var segments = components.path.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
      if segments.first == "" { segments.removeFirst() }
      guard segments.count == 2, segments[0] == "invite", !segments[1].isEmpty else { return nil }
      let relayScheme = components.scheme?.lowercased() == "https" ? "wss" : "ws"
      var relayComponents = URLComponents()
      relayComponents.scheme = relayScheme
      relayComponents.host = host
      relayComponents.port = components.port
      guard BuzzRelayOriginRules.isAcceptable(relayComponents, allowInsecureLocalhost: allowInsecureLocalhost) else {
        return nil
      }
      return BuzzInviteLink(
        relayURL: origin(scheme: relayScheme, host: host, port: components.port),
        code: segments[1],
        policyReceipt: nil,
        invocationURL: url
      )
    default:
      return nil
    }
  }

  static var isDebugBuild: Bool {
    #if DEBUG
      return true
    #else
      return false
    #endif
  }

  private static func hasUserInfo(_ components: URLComponents) -> Bool {
    !(components.user ?? "").isEmpty || !(components.password ?? "").isEmpty
  }

  /// Query parameters, rejecting a key that appears more than once (Dart's
  /// `queryParametersAll` check) by dropping it entirely.
  private static func singleValuedQuery(_ components: URLComponents) -> [String: String] {
    var seen: [String: Int] = [:]
    var values: [String: String] = [:]
    for item in components.queryItems ?? [] {
      seen[item.name, default: 0] += 1
      values[item.name] = item.value ?? ""
    }
    for (name, count) in seen where count > 1 { values[name] = nil }
    return values
  }

  private static func origin(scheme: String, host: String, port: Int?) -> String {
    var components = URLComponents()
    components.scheme = scheme
    components.host = host.lowercased()
    components.port = port
    return components.string ?? "\(scheme)://\(host.lowercased())"
  }
}

/// Mirror of `validateInviteRelayUri` in `lib/shared/relay/relay_validation.dart`.
///
/// Production relays must use TLS and may not be local or otherwise
/// non-public IP literals. DNS hostnames are accepted; pinning them to public
/// addresses is a connect-time concern the full app owns.
enum BuzzRelayOriginRules {
  static func isAcceptable(_ relay: URLComponents, allowInsecureLocalhost: Bool) -> Bool {
    guard let host = relay.host?.lowercased(), !host.isEmpty, (relay.user ?? "").isEmpty,
      (relay.password ?? "").isEmpty, relay.fragment == nil, relay.query == nil,
      relay.path.isEmpty || relay.path == "/"
    else { return false }
    guard let scheme = relay.scheme?.lowercased(), scheme == "ws" || scheme == "wss" else { return false }

    let isLocalhost = host == "localhost" || host.hasSuffix(".localhost")
    if scheme != "wss" && !(allowInsecureLocalhost && isLocalhost) { return false }
    if isLocalhost { return allowInsecureLocalhost }

    if let address = IPv4Address(dottedDecimal: host) { return !address.isNonPublic }
    if looksLikeNumericAddress(host) { return false }
    if let address = IPv6Address(literal: host) { return address.isPublic }
    return true
  }

  private static func looksLikeNumericAddress(_ host: String) -> Bool {
    func numeric(_ part: Substring) -> Bool {
      part.range(of: #"^\d+$"#, options: .regularExpression) != nil
        || part.range(of: #"^0x[0-9a-f]+$"#, options: [.regularExpression, .caseInsensitive]) != nil
    }
    if numeric(host[...]) { return true }
    let parts = host.split(separator: ".", omittingEmptySubsequences: false)
    return parts.count > 1 && parts.allSatisfy(numeric)
  }

  struct IPv4Address {
    let bytes: [UInt8]

    /// Only canonical dotted-decimal (four parts, no leading zeros) parses.
    init?(dottedDecimal host: String) {
      let parts = host.split(separator: ".", omittingEmptySubsequences: false)
      guard parts.count == 4 else { return nil }
      var bytes: [UInt8] = []
      for part in parts {
        guard !part.isEmpty, part.allSatisfy(\.isNumber), !(part.count > 1 && part.hasPrefix("0")),
          let value = UInt8(part)
        else { return nil }
        bytes.append(value)
      }
      self.bytes = bytes
    }

    init(bytes: [UInt8]) { self.bytes = bytes }

    var isNonPublic: Bool {
      guard bytes.count == 4 else { return true }
      let (a, b, c) = (bytes[0], bytes[1], bytes[2])
      return a == 0 || a == 10 || a == 127 || (a == 100 && (64...127).contains(b)) || (a == 169 && b == 254)
        || (a == 172 && (16...31).contains(b)) || (a == 192 && b == 0 && c == 0) || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168) || (a == 198 && (b == 18 || b == 19)) || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113) || a >= 224
    }
  }

  struct IPv6Address {
    let bytes: [UInt8]

    init?(literal host: String) {
      // `URLComponents.host` keeps the brackets around an IPv6 literal
      // (Dart's `Uri.host` strips them); `inet_pton` accepts neither.
      var literal = Substring(host)
      if literal.hasPrefix("["), literal.hasSuffix("]") { literal = literal.dropFirst().dropLast() }
      guard literal.contains(":") else { return nil }
      var raw = in6_addr()
      guard inet_pton(AF_INET6, String(literal), &raw) == 1 else { return nil }
      bytes = withUnsafeBytes(of: &raw) { Array($0) }
    }

    /// Mirrors `_isPublicAddress`: globally reachable unicast only, with the
    /// IANA special-purpose exclusions the Dart validator enumerates.
    var isPublic: Bool {
      guard bytes.count == 16 else { return false }
      let firstTenZero = bytes[0..<10].allSatisfy { $0 == 0 }
      if firstTenZero && ((bytes[10] == 0 && bytes[11] == 0) || (bytes[10] == 0xff && bytes[11] == 0xff)) {
        return !IPv4Address(bytes: Array(bytes[12..<16])).isNonPublic
      }
      guard hasPrefix([0x20], bits: 3) else { return false }
      if hasPrefix([0x20, 0x01, 0x00], bits: 23) && !isGloballyReachableIetfAssignment { return false }
      if hasPrefix([0x20, 0x01, 0x0d, 0xb8], bits: 32) { return false }
      if hasPrefix([0x20, 0x02], bits: 16) { return false }
      if hasPrefix([0x3f, 0xff, 0x00], bits: 20) { return false }
      return true
    }

    private var isGloballyReachableIetfAssignment: Bool {
      let isGlobalAnycast =
        hasPrefix([0x20, 0x01, 0x00, 0x01, 0, 0, 0, 0], bits: 64) && bytes[8..<15].allSatisfy { $0 == 0 }
        && (1...3).contains(bytes[15])
      return isGlobalAnycast || hasPrefix([0x20, 0x01, 0x00, 0x03], bits: 32)
        || hasPrefix([0x20, 0x01, 0x00, 0x04, 0x01, 0x12], bits: 48) || hasPrefix([0x20, 0x01, 0x00, 0x20], bits: 28)
        || hasPrefix([0x20, 0x01, 0x00, 0x30], bits: 28)
    }

    private func hasPrefix(_ prefix: [UInt8], bits: Int) -> Bool {
      let wholeBytes = bits / 8
      for index in 0..<wholeBytes where bytes[index] != prefix[index] { return false }
      let remaining = bits % 8
      if remaining == 0 { return true }
      let mask = UInt8(truncatingIfNeeded: (0xff << (8 - remaining)) & 0xff)
      return (bytes[wholeBytes] & mask) == (prefix[wholeBytes] & mask)
    }
  }
}
