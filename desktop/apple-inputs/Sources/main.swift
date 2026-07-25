import Foundation
import AppKit

func watchWorkspaceWake() {
    let center = NSWorkspace.shared.notificationCenter
    let observer = center.addObserver(
        forName: NSWorkspace.didWakeNotification,
        object: nil,
        queue: .main
    ) { _ in
        FileHandle.standardOutput.write(Data("workspace_did_wake\n".utf8))
    }
    withExtendedLifetime(observer) {
        RunLoop.main.run()
    }
    center.removeObserver(observer)
}

if CommandLine.arguments == [CommandLine.arguments[0], "--watch-workspace-wake"] {
    watchWorkspaceWake()
    exit(EXIT_SUCCESS)
}

let eventKit = EventKitReader()
let notes = NotesReader()
let fileAllowlist = [URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Documents")]

@MainActor
func response(for request: AppleInputRequest) async -> AppleInputResponse {
    do {
        switch request.payload {
        case .permission(let payload):
            if request.operation == .requestPermission {
                let state = await eventKit.requestPermission(source: payload.source)
                return .init(source: payload.source.rawValue, permission: state, records: [],
                             error: state == .authorized ? nil : "permission request was not granted")
            }
            let state: PermissionState
            switch payload.source {
            case .calendar, .reminders: state = eventKit.permissionStatus(source: payload.source)
            case .files: state = .authorized
            case .notes: state = .unavailable
            }
            return .init(source: payload.source.rawValue, permission: state, records: [],
                         error: state == .unavailable ? "permission status is unavailable without prompting" : nil)
        case .readCalendar(let payload):
            let page = try eventKit.readCalendar(calendarIdentifiers: payload.calendarIdentifiers, start: payload.start, end: payload.end, maximum: payload.maximum)
            return .init(source: "calendar", permission: eventKit.permissionStatus(source: .calendar),
                         records: page.records.map { $0.output() }, truncated: page.truncated)
        case .readReminders(let payload):
            let page = try eventKit.readReminders(listIdentifiers: payload.listIdentifiers, start: payload.start, end: payload.end, maximum: payload.maximum)
            return .init(source: "reminders", permission: eventKit.permissionStatus(source: .reminders),
                         records: page.records.map { $0.output() }, truncated: page.truncated)
        case .readNotes(let payload):
            let page = try notes.read(folderIdentifiers: payload.folderIdentifiers, maximum: payload.maximum)
            return .init(source: "notes", permission: .authorized, records: page.records.map { $0.output() }, truncated: page.truncated)
        case .readFiles(let payload):
            // Opening a protected user directory may invoke TCC. Keep it off
            // permission, protocol, and wake-monitor paths until files are read.
            let files = FileReader(allowlistedRoots: fileAllowlist)
            let page = try files.read(paths: payload.paths)
            return .init(source: "files", permission: .authorized, records: page.records.map { $0.output() }, truncated: page.truncated)
        }
    } catch {
        return .init(source: request.operation.rawValue, permission: .unavailable, records: [], error: error.localizedDescription)
    }
}

func protocolError(_ error: Error) -> String {
    let response = AppleInputResponse(source: "protocol", permission: .unavailable, records: [], error: error.localizedDescription)
    if let encoded = try? response.encodeLine() { return encoded }
    return #"{"source":"protocol","permission":"unavailable","observedAt":"","records":[],"truncated":false,"error":"protocol failure"}"# + "\n"
}

let reader = BoundedLineReader(handle: .standardInput)
while true {
    do {
        guard let line = try reader.nextLine() else { break }
        let output: String
        do { output = try await response(for: AppleInputRequest.decode(line: line)).encodeLine() }
        catch { output = protocolError(error) }
        FileHandle.standardOutput.write(Data(output.utf8))
    } catch {
        FileHandle.standardOutput.write(Data(protocolError(error).utf8))
    }
}
