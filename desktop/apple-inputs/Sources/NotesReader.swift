import Foundation

struct NoteRecord: Equatable { let identifier: String; let folderIdentifier: String; let title: String; let body: String
    func output() -> AppleInputRecord { .init(fields: ["identifier": identifier, "folder_identifier": folderIdentifier, "title": title, "body": body]) }
}

final class NotesReader {
    private let allowedFolders: Set<String>; private let fixtureRecords: [NoteRecord]?
    init(allowedFolders: Set<String> = ["Notes"]) { self.allowedFolders = allowedFolders; fixtureRecords = nil }
    private init(records: [NoteRecord]) { allowedFolders = Set(records.map(\.folderIdentifier)).union(["Notes", "work"]); fixtureRecords = records }
    static func fixture(records: [NoteRecord]) -> NotesReader { .init(records: records) }

    func read(folderIdentifiers: [String], maximum: Int) throws -> [NoteRecord] {
        let limit = try bounded(maximum, maximum: 100)
        guard Set(folderIdentifiers).isSubset(of: allowedFolders) else { throw AppleInputFailure.forbidden("requested Notes folder is not allowlisted") }
        if let fixtureRecords { return Array(fixtureRecords.filter { folderIdentifiers.contains($0.folderIdentifier) }.prefix(limit)) }
        let source = try fixedScript(folderIdentifiers: folderIdentifiers)
        var error: NSDictionary?
        guard let output = NSAppleScript(source: source)?.executeAndReturnError(&error).stringValue, error == nil else { throw AppleInputFailure.invalidRequest("Notes read failed") }
        return output.split(separator: "\n").prefix(limit).compactMap { line in let parts = line.split(separator: "\t", maxSplits: 2).map(String.init); guard parts.count == 3 else { return nil }; return NoteRecord(identifier: parts[0], folderIdentifier: folderIdentifiers.first ?? "Notes", title: parts[1], body: parts[2]) }
    }

    func fixedScript(folderIdentifiers: [String]) throws -> String {
        guard Set(folderIdentifiers).isSubset(of: allowedFolders) else { throw AppleInputFailure.forbidden("requested Notes folder is not allowlisted") }
        let folders = folderIdentifiers.map { "\"\($0.replacingOccurrences(of: "\\\"", with: "\\\\\\\""))\"" }.joined(separator: ", ")
        return """
        tell application "Notes"
          set resultRows to {}
          repeat with folderName in {(folders)}
            repeat with n in notes of folder folderName
              set end of resultRows to (id of n as text) & tab & (name of n as text) & tab & (body of n as text)
            end repeat
          end repeat
          return resultRows as text
        end tell
        """
    }
}
