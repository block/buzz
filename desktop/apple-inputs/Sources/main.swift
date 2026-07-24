import Foundation

let eventKit = EventKitReader()
let notes = NotesReader()
let defaultFiles = FileReader(allowlistedRoots: [URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Documents")])

@MainActor
func response(for request: AppleInputRequest) async -> AppleInputResponse {
    do {
        switch request.operation {
        case .permissionStatus:
            let state = eventKit.permissionStatus()
            return AppleInputResponse(source: "eventkit", permission: state.calendar, records: [.init(fields: ["calendar": state.calendar.rawValue, "reminders": state.reminders.rawValue])])
        case .requestPermission:
            let state = await eventKit.requestPermission()
            return AppleInputResponse(source: "eventkit", permission: state.calendar, records: [.init(fields: ["calendar": state.calendar.rawValue, "reminders": state.reminders.rawValue])])
        case .readCalendar:
            let ids = request.arguments["calendar_ids"] as? [String] ?? []; let max = try bounded(request.arguments["maximum"] as? Int, maximum: 100)
            let values = try eventKit.readCalendar(calendarIdentifiers: ids, start: Date().addingTimeInterval(-86_400 * 30), end: Date().addingTimeInterval(86_400 * 90), maximum: max)
            return AppleInputResponse(source: "calendar", permission: eventKit.permissionStatus().calendar, records: values.map { $0.output() }, truncated: values.count == max)
        case .readReminders:
            let ids = request.arguments["list_ids"] as? [String] ?? []; let max = try bounded(request.arguments["maximum"] as? Int, maximum: 100)
            let values = try eventKit.readReminders(listIdentifiers: ids, maximum: max)
            return AppleInputResponse(source: "reminders", permission: eventKit.permissionStatus().reminders, records: values.map { $0.output() }, truncated: values.count == max)
        case .readNotes:
            let folders = request.arguments["folder_ids"] as? [String] ?? []; let max = try bounded(request.arguments["maximum"] as? Int, maximum: 100)
            let values = try notes.read(folderIdentifiers: folders, maximum: max)
            return AppleInputResponse(source: "notes", permission: .authorized, records: values.map { $0.output() }, truncated: values.count == max)
        case .readFiles:
            let paths = request.arguments["paths"] as? [String] ?? []
            return AppleInputResponse(source: "files", permission: .authorized, records: try defaultFiles.read(paths: paths).map { $0.output() })
        }
    } catch { return AppleInputResponse(source: request.operation.rawValue, permission: .unavailable, records: [], error: error.localizedDescription) }
}

while let line = readLine() {
    let output: String
    do { output = try await response(for: AppleInputRequest.decode(line: line)).encodeLine() }
    catch { output = try! AppleInputResponse(source: "protocol", permission: .unavailable, records: [], error: error.localizedDescription).encodeLine() }
    FileHandle.standardOutput.write(output.data(using: .utf8)!)
}
