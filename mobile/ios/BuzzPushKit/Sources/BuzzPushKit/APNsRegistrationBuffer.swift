import Foundation

public struct APNsRegistrationUpdate: Equatable, Sendable {
    public let method: String
    public let arguments: [String: String]
    public init(method: String, arguments: [String: String]) {
        self.method = method
        self.arguments = arguments
    }
}

public final class APNsRegistrationBuffer {
    public private(set) var pending: APNsRegistrationUpdate?
    private var deliver: ((APNsRegistrationUpdate) -> Void)?
    public init() {}
    public func attach(_ deliver: @escaping (APNsRegistrationUpdate) -> Void) {
        self.deliver = deliver
        flush()
    }
    public func recordToken(_ token: Data) {
        record(APNsRegistrationUpdate(
            method: "apnsTokenChanged",
            arguments: ["token": token.map { String(format: "%02x", $0) }.joined()]
        ))
    }
    public func recordError(_ message: String) {
        record(APNsRegistrationUpdate(
            method: "apnsRegistrationFailed", arguments: ["message": message]
        ))
    }
    private func record(_ update: APNsRegistrationUpdate) {
        pending = update
        flush()
    }
    private func flush() {
        guard let pending, let deliver else { return }
        self.pending = nil
        deliver(pending)
    }
}
