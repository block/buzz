import XCTest

final class FileReaderTests: XCTestCase {
    func testReadsRegularFileInsideCanonicalAllowlist() throws {
        let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("brief.txt")
        try Data("safe".utf8).write(to: file)
        let reader = FileReader(allowlistedRoots: [root], maximumBytes: 64, maximumFiles: 1)
        XCTAssertEqual(try reader.read(paths: [file.path]).first?.contents, "safe")
    }

    func testRejectsSymlinkEvenWhenItAppearsInsideAllowlist() throws {
        let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let link = root.appendingPathComponent("escape")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: URL(fileURLWithPath: "/etc/hosts"))
        let reader = FileReader(allowlistedRoots: [root], maximumBytes: 64, maximumFiles: 1)
        XCTAssertThrowsError(try reader.read(paths: [link.path]))
    }
}
