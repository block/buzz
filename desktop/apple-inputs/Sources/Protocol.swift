import Foundation

enum ProtocolLimits {
    static let maximumLineBytes = 64 * 1024
    static let maximumResponseBytes = 1024 * 1024
    static let maximumArrayCount = 32
    static let maximumStringBytes = 1024
    static let maximumRecords = 100
    static let maximumWindow: TimeInterval = 366 * 86_400
}

enum AppleInputOperation: String, Codable {
    case permissionStatus = "permission_status", requestPermission = "request_permission"
    case readCalendar = "read_calendar", readReminders = "read_reminders"
    case readNotes = "read_notes", readFiles = "read_files"
}
enum PermissionSource: String { case calendar, reminders, notes, files }
enum PermissionState: String, Codable { case notDetermined = "not_determined", denied, authorized, restricted, unavailable }
enum AppleInputFailure: LocalizedError {
    case invalidRequest(String), forbidden(String), invalidBound
    var errorDescription: String? {
        switch self {
        case .invalidRequest(let message), .forbidden(let message): message
        case .invalidBound: "requested bound is outside the helper limit"
        }
    }
}

struct PermissionPayload { let source: PermissionSource }
struct CalendarPayload { let calendarIdentifiers: [String]; let start: Date; let end: Date; let maximum: Int }
struct ReminderPayload { let listIdentifiers: [String]; let start: Date; let end: Date; let maximum: Int }
struct NotesPayload { let folderIdentifiers: [String]; let maximum: Int }
struct FilesPayload { let paths: [String] }
enum AppleInputPayload {
    case permission(PermissionPayload), readCalendar(CalendarPayload), readReminders(ReminderPayload), readNotes(NotesPayload), readFiles(FilesPayload)
}
struct AppleInputRequest {
    let operation: AppleInputOperation
    let payload: AppleInputPayload

    static func decode(line: String) throws -> AppleInputRequest {
        guard line.utf8.count <= ProtocolLimits.maximumLineBytes, let data = line.data(using: .utf8),
              let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw AppleInputFailure.invalidRequest("invalid or oversized JSON request")
        }
        try exact(root, keys: ["operation", "arguments"])
        guard let raw = root["operation"] as? String, let operation = AppleInputOperation(rawValue: raw),
              let arguments = root["arguments"] as? [String: Any] else {
            throw AppleInputFailure.invalidRequest("operation and arguments are required")
        }
        switch operation {
        case .permissionStatus, .requestPermission:
            try exact(arguments, keys: ["source"])
            guard let rawSource = arguments["source"] as? String, let source = PermissionSource(rawValue: rawSource) else {
                throw AppleInputFailure.invalidRequest("source must be a known string")
            }
            if operation == .requestPermission, source != .calendar, source != .reminders {
                throw AppleInputFailure.invalidRequest("permission can only be requested for calendar or reminders")
            }
            return .init(operation: operation, payload: .permission(.init(source: source)))
        case .readCalendar:
            try exact(arguments, keys: ["calendar_ids", "start", "end", "maximum"])
            let ids = try stringArray(arguments["calendar_ids"], name: "calendar_ids")
            let start = try date(arguments["start"], name: "start"), end = try date(arguments["end"], name: "end")
            guard start < end, end.timeIntervalSince(start) <= ProtocolLimits.maximumWindow else {
                throw AppleInputFailure.invalidRequest("calendar window must be ascending and at most 366 days")
            }
            return .init(operation: operation, payload: .readCalendar(.init(calendarIdentifiers: ids, start: start, end: end, maximum: try maximum(arguments["maximum"]))))
        case .readReminders:
            try exact(arguments, keys: ["list_ids", "start", "end", "maximum"])
            let start = try date(arguments["start"], name: "start"), end = try date(arguments["end"], name: "end")
            guard start < end, end.timeIntervalSince(start) <= ProtocolLimits.maximumWindow else {
                throw AppleInputFailure.invalidRequest("reminder window must be ascending and at most 366 days")
            }
            return .init(operation: operation, payload: .readReminders(.init(listIdentifiers: try stringArray(arguments["list_ids"], name: "list_ids"), start: start, end: end, maximum: try maximum(arguments["maximum"]))))
        case .readNotes:
            try exact(arguments, keys: ["folder_ids", "maximum"])
            return .init(operation: operation, payload: .readNotes(.init(folderIdentifiers: try stringArray(arguments["folder_ids"], name: "folder_ids"), maximum: try maximum(arguments["maximum"]))))
        case .readFiles:
            try exact(arguments, keys: ["paths"])
            return .init(operation: operation, payload: .readFiles(.init(paths: try stringArray(arguments["paths"], name: "paths"))))
        }
    }

    private static func exact(_ object: [String: Any], keys: Set<String>) throws {
        guard Set(object.keys) == keys else { throw AppleInputFailure.invalidRequest("object has missing or unknown keys") }
    }
    private static func stringArray(_ value: Any?, name: String) throws -> [String] {
        guard let strings = value as? [String], !strings.isEmpty, strings.count <= ProtocolLimits.maximumArrayCount,
              strings.allSatisfy({ !$0.isEmpty && $0.utf8.count <= ProtocolLimits.maximumStringBytes }) else {
            throw AppleInputFailure.invalidRequest("\(name) must be a bounded non-empty string array")
        }
        return strings
    }
    private static func maximum(_ value: Any?) throws -> Int {
        guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else { throw AppleInputFailure.invalidRequest("maximum must be an integer") }
        let integer = number.intValue
        guard integer > 0, integer <= ProtocolLimits.maximumRecords, Double(integer) == number.doubleValue else { throw AppleInputFailure.invalidBound }
        return integer
    }
    private static func date(_ value: Any?, name: String) throws -> Date {
        guard let text = value as? String, text.utf8.count <= 64, let parsed = ISO8601DateFormatter().date(from: text) else {
            throw AppleInputFailure.invalidRequest("\(name) must be an ISO-8601 date")
        }
        return parsed
    }
}

struct AppleInputRecord: Codable, Equatable { let fields: [String: String] }
struct AppleInputResponse: Codable {
    let source: String, observedAt: String
    let permission: PermissionState
    let records: [AppleInputRecord]
    let truncated: Bool
    let error: String?
    init(source: String, permission: PermissionState, records: [AppleInputRecord], truncated: Bool = false, error: String? = nil) {
        self.source = source; self.permission = permission; self.records = records; self.truncated = truncated; self.error = error
        observedAt = ISO8601DateFormatter().string(from: Date())
    }
    func encodeLine() throws -> String {
        let encoder = JSONEncoder()
        let data = try encoder.encode(self)
        guard data.count + 1 <= ProtocolLimits.maximumResponseBytes, let text = String(data: data, encoding: .utf8), !text.contains("\n") else {
            throw AppleInputFailure.invalidRequest("response exceeds protocol bound")
        }
        return text + "\n"
    }
    enum CodingKeys: String, CodingKey { case source, permission, observedAt, records, truncated, error }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(source, forKey: .source); try c.encode(permission, forKey: .permission); try c.encode(observedAt, forKey: .observedAt)
        try c.encode(records, forKey: .records); try c.encode(truncated, forKey: .truncated)
        if let error { try c.encode(error, forKey: .error) } else { try c.encodeNil(forKey: .error) }
    }
}

final class BoundedLineReader {
    private let handle: FileHandle
    private var buffer = Data()
    init(handle: FileHandle) { self.handle = handle }
    func nextLine() throws -> String? {
        while true {
            if let newline = buffer.firstIndex(of: 0x0A) {
                let line = buffer.prefix(upTo: newline)
                buffer.removeSubrange(...newline)
                guard line.count <= ProtocolLimits.maximumLineBytes else { throw AppleInputFailure.invalidRequest("request line exceeds protocol bound") }
                return String(decoding: line, as: UTF8.self)
            }
            guard let chunk = try handle.read(upToCount: 4096), !chunk.isEmpty else {
                if buffer.isEmpty { return nil }
                defer { buffer.removeAll() }
                guard buffer.count <= ProtocolLimits.maximumLineBytes else { throw AppleInputFailure.invalidRequest("request line exceeds protocol bound") }
                return String(decoding: buffer, as: UTF8.self)
            }
            buffer.append(chunk)
            if buffer.count > ProtocolLimits.maximumLineBytes, !buffer.contains(0x0A) {
                while let more = try handle.read(upToCount: 4096), !more.isEmpty {
                    if more.contains(0x0A) { break }
                }
                buffer.removeAll()
                throw AppleInputFailure.invalidRequest("request line exceeds protocol bound")
            }
        }
    }
}
