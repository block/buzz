import XCTest

final class ProtocolTests: XCTestCase {
    func testDecodesOnlyKnownOperationAndEmitsOneLineResponse() throws {
        let request = try AppleInputRequest.decode(line: #"{"operation":"permission_status","arguments":{"source":"calendar"}}"#)
        XCTAssertEqual(request.operation, .permissionStatus)
        let output = try AppleInputResponse(source: "calendar", permission: .notDetermined, records: []).encodeLine()
        XCTAssertFalse(output.dropLast().contains("\n"))
        XCTAssertTrue(output.hasSuffix("\n"))
    }

    func testRejectsUnknownRequestKeys() {
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_notes","script":"tell application \"Finder\""}"#))
    }

    func testDecodesTypedCalendarWindow() throws {
        let request = try AppleInputRequest.decode(line: #"{"operation":"read_calendar","arguments":{"calendar_ids":["work"],"start":"2026-07-01T00:00:00Z","end":"2026-07-02T00:00:00Z","maximum":25}}"#)
        guard case .readCalendar(let payload) = request.payload else { return XCTFail("wrong payload") }
        XCTAssertEqual(payload.calendarIdentifiers, ["work"])
        XCTAssertEqual(payload.maximum, 25)
    }

    func testReminderPayloadRequiresBoundedRFC3339Window() throws {
        let request = try AppleInputRequest.decode(line: #"{"operation":"read_reminders","arguments":{"list_ids":["work"],"start":"2026-07-01T00:00:00Z","end":"2026-07-02T00:00:00Z","maximum":2}}"#)
        guard case .readReminders(let payload) = request.payload else { return XCTFail("wrong payload") }
        XCTAssertLessThan(payload.start, payload.end)
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_reminders","arguments":{"list_ids":["work"],"maximum":2}}"#))
    }

    func testRejectsWrongTypesUnknownPayloadKeysAndOversizedIdentifiers() {
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_files","arguments":{"paths":"nope"}}"#))
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_notes","arguments":{"folder_ids":["work"],"maximum":1,"script":"bad"}}"#))
        let long = String(repeating: "x", count: ProtocolLimits.maximumStringBytes + 1)
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_files","arguments":{"paths":["\#(long)"]}}"#))
    }

    func testRejectsExcessiveCalendarWindowAndResponseSize() throws {
        XCTAssertThrowsError(try AppleInputRequest.decode(line: #"{"operation":"read_calendar","arguments":{"calendar_ids":["work"],"start":"2020-01-01T00:00:00Z","end":"2026-01-01T00:00:00Z","maximum":1}}"#))
        let huge = AppleInputRecord(fields: ["body": String(repeating: "x", count: ProtocolLimits.maximumResponseBytes)])
        XCTAssertThrowsError(try AppleInputResponse(source: "notes", permission: .authorized, records: [huge]).encodeLine())
    }

    func testSubprocessRejectsMalformedAndOversizedLinesWithoutCrashing() throws {
        let malformed = try runHelper(input: "{bad}\n")
        XCTAssertTrue(malformed.contains(#""source":"protocol""#))
        XCTAssertTrue(malformed.contains(#""error":"#))

        let oversized = String(repeating: "x", count: ProtocolLimits.maximumLineBytes + 1) + "\n"
        let response = try runHelper(input: oversized)
        XCTAssertTrue(response.contains(#""source":"protocol""#))
    }

    private func runHelper(input: String) throws -> String {
        let executable = Bundle(for: Self.self).bundleURL
            .deletingLastPathComponent().appendingPathComponent("BuzzAppleInputs")
        let process = Process()
        process.executableURL = executable
        let stdin = Pipe(), stdout = Pipe()
        process.standardInput = stdin; process.standardOutput = stdout
        try process.run()
        stdin.fileHandleForWriting.write(Data(input.utf8))
        try stdin.fileHandleForWriting.close()
        process.waitUntilExit()
        return String(decoding: stdout.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    }
}
