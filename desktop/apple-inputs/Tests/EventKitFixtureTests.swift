import XCTest

final class EventKitFixtureTests: XCTestCase {
    func testPermissionStatusNeverRequestsFixturePermission() {
        let reader = EventKitReader.fixture(calendarPermission: .notDetermined, remindersPermission: .denied)
        XCTAssertEqual(reader.permissionStatus().calendar, .notDetermined)
        XCTAssertEqual(reader.permissionStatus().reminders, .denied)
        XCTAssertEqual(reader.requestCount, 0)
    }

    func testWriteOnlyIsNotReadAuthorized() {
        XCTAssertEqual(EventKitReader.permissionState(for: .writeOnly), .denied)
        XCTAssertEqual(EventKitReader.permissionState(for: .fullAccess), .authorized)
    }

    func testCalendarFixtureHonoursAllowlistAndBound() throws {
        let reader = EventKitReader.fixture(calendarRecords: [
            .init(identifier: "included", calendarIdentifier: "work", title: "Brief", recurrenceIdentifier: "r1", start: .distantPast, end: .distantFuture, isRecurring: true, isDeleted: false, isStale: false),
            .init(identifier: "excluded", calendarIdentifier: "personal", title: "Private", recurrenceIdentifier: "", start: .distantPast, end: .distantFuture, isRecurring: false, isDeleted: false, isStale: false),
        ])
        let page = try reader.readCalendar(calendarIdentifiers: ["work"], start: Date(), end: Date().addingTimeInterval(86_400), maximum: 1)
        XCTAssertEqual(page.records.map(\.identifier), ["included"])
    }
}
