import Foundation

struct NoteRecord: Equatable {
    let identifier, folderIdentifier, title, body: String
    func output() -> AppleInputRecord { .init(fields: ["identifier": identifier, "folder_identifier": folderIdentifier, "title": title, "body": body]) }
}
final class NotesReader {
    private static let maximumTitleCharacters = 512
    private static let maximumBodyCharacters = 32_768
    private let allowedFolders: Set<String>
    private let fixtureRecords: [NoteRecord]?
    init(allowedFolders: Set<String> = ["Notes"]) { self.allowedFolders = allowedFolders; fixtureRecords = nil }
    private init(records: [NoteRecord]) { allowedFolders = Set(records.map(\.folderIdentifier)).union(["Notes", "work"]); fixtureRecords = records }
    static func fixture(records: [NoteRecord]) -> NotesReader { .init(records: records) }

    func read(folderIdentifiers: [String], maximum: Int) throws -> Page<NoteRecord> {
        try validate(folderIdentifiers)
        if let fixtureRecords {
            let selected = fixtureRecords.filter { folderIdentifiers.contains($0.folderIdentifier) }
            return .init(records: Array(selected.prefix(maximum)), truncated: selected.count > maximum)
        }
        let source = try fixedScript(folderIdentifiers: folderIdentifiers, maximum: maximum)
        var error: NSDictionary?
        guard let descriptor = NSAppleScript(source: source)?.executeAndReturnError(&error), error == nil else {
            throw AppleInputFailure.invalidRequest("Notes read failed")
        }
        let count = descriptor.numberOfItems
        var records: [NoteRecord] = []
        records.reserveCapacity(min(count, maximum))
        for index in 1...min(count, maximum) {
            guard let row = descriptor.atIndex(index), row.numberOfItems == 4,
                  let identifier = row.atIndex(1)?.stringValue,
                  let folder = row.atIndex(2)?.stringValue,
                  let title = row.atIndex(3)?.stringValue,
                  let body = row.atIndex(4)?.stringValue else { continue }
            records.append(.init(identifier: String(identifier.prefix(ProtocolLimits.maximumStringBytes)),
                                 folderIdentifier: String(folder.prefix(ProtocolLimits.maximumStringBytes)),
                                 title: String(title.prefix(Self.maximumTitleCharacters)),
                                 body: String(body.prefix(Self.maximumBodyCharacters))))
        }
        return .init(records: records, truncated: count > maximum)
    }

    func fixedScript(folderIdentifiers: [String], maximum: Int) throws -> String {
        try validate(folderIdentifiers)
        guard maximum > 0, maximum <= ProtocolLimits.maximumRecords else { throw AppleInputFailure.invalidBound }
        let folders = folderIdentifiers.map(Self.appleScriptLiteral).joined(separator: ", ")
        return """
        set maximumTitleCharacters to \(Self.maximumTitleCharacters)
        set maximumBodyCharacters to \(Self.maximumBodyCharacters)
        set maximumRecords to \(maximum + 1)
        tell application "Notes"
          set resultRows to {}
          repeat with folderName in {\(folders)}
            set noteLimit to count of notes of folder (folderName as text)
            if noteLimit > maximumRecords - (count of resultRows) then set noteLimit to maximumRecords - (count of resultRows)
            if noteLimit > 0 then
              repeat with noteIndex from 1 to noteLimit
                set n to item noteIndex of notes of folder (folderName as text)
                set noteTitle to name of n as text
                set noteBody to body of n as text
                if (count of noteTitle) > maximumTitleCharacters then set noteTitle to text 1 thru maximumTitleCharacters of noteTitle
                if (count of noteBody) > maximumBodyCharacters then set noteBody to text 1 thru maximumBodyCharacters of noteBody
                set end of resultRows to {(id of n as text), (folderName as text), noteTitle, noteBody}
              end repeat
            end if
            if (count of resultRows) ≥ maximumRecords then exit repeat
          end repeat
          return resultRows
        end tell
        """
    }
    private func validate(_ folders: [String]) throws {
        guard !folders.isEmpty, folders.count <= ProtocolLimits.maximumArrayCount,
              folders.allSatisfy({ !$0.isEmpty && $0.utf8.count <= ProtocolLimits.maximumStringBytes }),
              Set(folders).isSubset(of: allowedFolders) else {
            throw AppleInputFailure.forbidden("requested Notes folder is not allowlisted")
        }
    }
    private static func appleScriptLiteral(_ value: String) -> String {
        let escaped = value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        return "\"\(escaped)\""
    }
}
