import Foundation

enum AppleInputOperation: String, Codable, Equatable {
    case permissionStatus = "permission_status"
    case requestPermission = "request_permission"
    case readCalendar = "read_calendar"
    case readReminders = "read_reminders"
    case readNotes = "read_notes"
    case readFiles = "read_files"
}

enum PermissionState: String, Codable, Equatable { case notDetermined = "not_determined", denied, authorized, restricted, unavailable }

enum AppleInputFailure: LocalizedError { case invalidRequest(String), forbidden(String), invalidBound
    var errorDescription: String? { switch self { case .invalidRequest(let value), .forbidden(let value): value; case .invalidBound: "requested bound is outside the helper limit" } }
}

struct AppleInputRequest {
    let operation: AppleInputOperation
    let arguments: [String: Any]

    static func decode(line: String) throws -> AppleInputRequest {
        guard let data = line.data(using: .utf8), let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys).isSubset(of: ["operation", "arguments"]), let raw = object["operation"] as? String,
              let operation = AppleInputOperation(rawValue: raw) else { throw AppleInputFailure.invalidRequest("request must contain only a known operation and arguments") }
        guard object["arguments"] == nil || object["arguments"] is [String: Any] else { throw AppleInputFailure.invalidRequest("arguments must be an object") }
        return .init(operation: operation, arguments: object["arguments"] as? [String: Any] ?? [:])
    }
}

struct AppleInputRecord: Codable, Equatable { let fields: [String: String] }

struct AppleInputResponse: Codable {
    let source: String
    let permission: PermissionState
    let observedAt: String
    let records: [AppleInputRecord]
    let truncated: Bool
    let error: String?

    init(source: String, permission: PermissionState, records: [AppleInputRecord], truncated: Bool = false, error: String? = nil) {
        self.source = source; self.permission = permission; self.records = records; self.truncated = truncated; self.error = error
        self.observedAt = ISO8601DateFormatter().string(from: Date())
    }

    func encodeLine() throws -> String {
        let data = try JSONEncoder().encode(self)
        guard let text = String(data: data, encoding: .utf8), !text.contains("\n") else { throw AppleInputFailure.invalidRequest("response cannot be encoded as one line") }
        return text + "\n"
    }

    enum CodingKeys: String, CodingKey { case source, permission, observedAt, records, truncated, error }
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(source, forKey: .source); try container.encode(permission, forKey: .permission); try container.encode(observedAt, forKey: .observedAt)
        try container.encode(records, forKey: .records); try container.encode(truncated, forKey: .truncated)
        if let error { try container.encode(error, forKey: .error) } else { try container.encodeNil(forKey: .error) }
    }
}

func bounded(_ requested: Int?, maximum: Int) throws -> Int {
    let value = requested ?? maximum
    guard value > 0, value <= maximum else { throw AppleInputFailure.invalidBound }
    return value
}
