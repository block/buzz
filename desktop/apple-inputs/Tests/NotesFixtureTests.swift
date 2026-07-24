import XCTest

final class NotesFixtureTests: XCTestCase {
    func testNotesFixtureOnlyReadsConfiguredFolders() throws {
        let reader = NotesReader.fixture(records: [.init(identifier: "1", folderIdentifier: "work", title: "Plan", body: "bounded")])
        XCTAssertEqual(try reader.read(folderIdentifiers: ["work"], maximum: 1).map(\.title), ["Plan"])
        XCTAssertThrowsError(try reader.read(folderIdentifiers: ["other"], maximum: 1))
    }

    func testAppleScriptIsFixedToNotesApplication() throws {
        let reader = NotesReader.fixture(records: [])
        let script = try reader.fixedScript(folderIdentifiers: ["work"])
        XCTAssertTrue(script.contains("application \"Notes\""))
        XCTAssertFalse(script.contains("Finder"))
    }
}
