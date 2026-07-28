import Foundation

enum ProtocolLimits {
    static let maximumLineBytes = 8 * 1024 * 1024
    static let maximumResponseBytes = 1024 * 1024
    static let maximumArrayCount = 32
    static let maximumStringBytes = 1024
    static let maximumRecords = 100
    static let maximumCalendarProjections = 2_000
    static let maximumWindow: TimeInterval = 366 * 86_400
}

enum AppleInputOperation: String, Codable {
    case permissionStatus = "permission_status", requestPermission = "request_permission"
    case listCalendars = "list_calendars", listReminderLists = "list_reminder_lists"
    case listNoteFolders = "list_note_folders"
    case readCalendar = "read_calendar", readReminders = "read_reminders"
    case readNotes = "read_notes", readFiles = "read_files"
    case extractPDF = "extract_pdf"
    case reconcileCalendar = "reconcile_calendar"
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
struct PDFPayload { let path: String }
struct ReconcileCalendarPayload {
    let coverageStart: Date
    let coverageEnd: Date
    let projections: [CalendarProjection]
}
enum AppleInputPayload {
    case permission(PermissionPayload), listCalendars, listReminderLists, listNoteFolders
    case readCalendar(CalendarPayload), readReminders(ReminderPayload), readNotes(NotesPayload), readFiles(FilesPayload)
    case extractPDF(PDFPayload)
    case reconcileCalendar(ReconcileCalendarPayload)
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
        case .listCalendars:
            try exact(arguments, keys: [])
            return .init(operation: operation, payload: .listCalendars)
        case .listReminderLists:
            try exact(arguments, keys: [])
            return .init(operation: operation, payload: .listReminderLists)
        case .listNoteFolders:
            try exact(arguments, keys: [])
            return .init(operation: operation, payload: .listNoteFolders)
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
        case .extractPDF:
            try exact(arguments, keys: ["path"])
            guard let path = arguments["path"] as? String, !path.isEmpty,
                  path.utf8.count <= 4096, path.hasPrefix("/") else {
                throw AppleInputFailure.invalidRequest("path must be a bounded absolute path")
            }
            return .init(operation: operation, payload: .extractPDF(.init(path: path)))
        case .reconcileCalendar:
            try exact(arguments, keys: ["coverage_start", "coverage_end", "projections"])
            let coverageStart = try date(arguments["coverage_start"], name: "coverage_start")
            let coverageEnd = try date(arguments["coverage_end"], name: "coverage_end")
            guard coverageStart < coverageEnd,
                  coverageEnd.timeIntervalSince(coverageStart) <= 5 * 366 * 86_400 else {
                throw AppleInputFailure.invalidRequest("calendar publication coverage is invalid")
            }
            guard let rawProjections = arguments["projections"] as? [[String: Any]],
                  rawProjections.count <= ProtocolLimits.maximumCalendarProjections else {
                throw AppleInputFailure.invalidRequest("calendar projections exceed the bounded limit")
            }
            let projections = try rawProjections.map { value in
                try exact(value, keys: [
                    "external_id", "title", "start", "end", "is_all_day",
                    "location", "notes",
                ])
                guard let externalID = value["external_id"] as? String,
                      externalID.hasPrefix("battle-rhythm:"),
                      !externalID.isEmpty,
                      externalID.utf8.count <= 512,
                      let title = value["title"] as? String,
                      !title.isEmpty,
                      title.utf8.count <= 512,
                      let isAllDay = value["is_all_day"] as? Bool else {
                    throw AppleInputFailure.invalidRequest("calendar projection fields are invalid")
                }
                let start = try date(value["start"], name: "projection start")
                let end = try date(value["end"], name: "projection end")
                guard start < end, start < coverageEnd, end > coverageStart else {
                    throw AppleInputFailure.invalidRequest("calendar projection is outside coverage")
                }
                return CalendarProjection(
                    externalID: externalID,
                    title: title,
                    start: start,
                    end: end,
                    isAllDay: isAllDay,
                    location: try optionalString(value["location"], name: "location", maximum: 1_024),
                    notes: try optionalString(value["notes"], name: "notes", maximum: 4_096)
                )
            }
            guard Set(projections.map(\.externalID)).count == projections.count else {
                throw AppleInputFailure.invalidRequest("calendar projection IDs must be unique")
            }
            return .init(
                operation: operation,
                payload: .reconcileCalendar(.init(
                    coverageStart: coverageStart,
                    coverageEnd: coverageEnd,
                    projections: projections
                ))
            )
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
    private static func optionalString(
        _ value: Any?,
        name: String,
        maximum: Int
    ) throws -> String? {
        if value is NSNull { return nil }
        guard let text = value as? String,
              !text.isEmpty,
              text.utf8.count <= maximum,
              !text.contains("\0") else {
            throw AppleInputFailure.invalidRequest("\(name) must be null or bounded text")
        }
        return text
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
