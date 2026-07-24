import XCTest

final class ProtocolTests: XCTestCase {
    func testDecodesOnlyKnownOperationAndEmitsOneLineResponse() throws {
        let request = try AppleInputRequest.decode(line: #"{"operation":"permission_status"}"#)
        XCTAssertEqual(request.operation, .permissionStatus)
        let output = try AppleInputResponse(source: "calendar", permission: .notDetermined, records: []).encodeLine()
        XCTAssertFalse(output.dropLast().contains("\n"))
        XCTAssertTrue(output.hasSuffix("\n"))
    }

    func testRejectsUnknownRequestKeys() {
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_notes","script":"tell application \"Finder\""}"#))
    }
}
