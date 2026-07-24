import XCTest

final class EventKitFixtureTests: XCTestCase {
    func testPermissionStatusNeverRequestsFixturePermission() {
        let reader = EventKitReader.fixture(calendarPermission: .notDetermined, remindersPermission: .denied)
        XCTAssertEqual(reader.permissionStatus().calendar, .notDetermined)
        XCTAssertEqual(reader.permissionStatus().reminders, .denied)
        XCTAssertEqual(reader.requestCount, 0)
    }

    func testCalendarFixtureHonoursAllowlistAndBound() throws {
        let reader = EventKitReader.fixture(calendarRecords: [
            .init(identifier: "included", calendarIdentifier: "work", title: "Brief", start: .distantPast, end: .distantFuture, isRecurring: true),
            .init(identifier: "excluded", calendarIdentifier: "personal", title: "Private", start: .distantPast, end: .distantFuture, isRecurring: false),
        ])
        let records = try reader.readCalendar(calendarIdentifiers: ["work"], start: .distantPast, end: .distantFuture, maximum: 1)
        XCTAssertEqual(records.map(\.identifier), ["included"])
    }
}
