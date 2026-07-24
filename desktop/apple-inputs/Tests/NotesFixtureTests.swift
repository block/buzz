import XCTest

final class NotesFixtureTests: XCTestCase {
    func testNotesFixtureOnlyReadsConfiguredFolders() throws {
        let reader = NotesReader.fixture(records: [.init(identifier: "1", folderIdentifier: "work", title: "Plan", body: "bounded")])
        XCTAssertEqual(try reader.read(folderIdentifiers: ["work"], maximum: 1).records.map(\.title), ["Plan"])
        XCTAssertThrowsError(try reader.read(folderIdentifiers: ["other"], maximum: 1))
    }

    func testAppleScriptIsFixedToNotesApplication() throws {
        let reader = NotesReader.fixture(records: [])
        let script = try reader.fixedScript(folderIdentifiers: ["work"], maximum: 1)
        XCTAssertTrue(script.contains("application \"Notes\""))
        XCTAssertFalse(script.contains("Finder"))
    }

    func testScriptEscapesBareQuotesAndAppliesSourceLimit() throws {
        let reader = NotesReader(allowedFolders: [#"work"quotes"#])
        let script = try reader.fixedScript(folderIdentifiers: [#"work"quotes"#], maximum: 2)
        XCTAssertTrue(script.contains(#"work\"quotes"#))
        XCTAssertTrue(script.contains("repeat with noteIndex from 1 to noteLimit"))
        XCTAssertTrue(script.contains("maximumTitleCharacters"))
        XCTAssertTrue(script.contains("maximumBodyCharacters"))
    }

    func testEmptyNotesDescriptorReturnsNoRecordsWithoutTrapping() throws {
        let reader = NotesReader(allowedFolders: ["work"])
        let page = try reader.records(from: NSAppleEventDescriptor.list(), maximum: 2)
        XCTAssertTrue(page.records.isEmpty)
        XCTAssertFalse(page.truncated)
    }
}
