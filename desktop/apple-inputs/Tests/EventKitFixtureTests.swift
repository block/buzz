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

    func testReminderFixtureFiltersIncompleteDueAndCompletedCompletionDatesAndDedupes() throws {
        let start = Date(timeIntervalSince1970: 1_000)
        let end = Date(timeIntervalSince1970: 2_000)
        let reader = EventKitReader.fixture(reminderRecords: [
            .init(identifier: "due", listIdentifier: "work", title: "Due", recurrenceIdentifier: "", dueDate: Date(timeIntervalSince1970: 1_500), completionDate: nil, isCompleted: false, isDeleted: false, isStale: false),
            .init(identifier: "done", listIdentifier: "work", title: "Done", recurrenceIdentifier: "", dueDate: Date(timeIntervalSince1970: 500), completionDate: Date(timeIntervalSince1970: 1_600), isCompleted: true, isDeleted: false, isStale: false),
            .init(identifier: "due", listIdentifier: "work", title: "Duplicate", recurrenceIdentifier: "", dueDate: Date(timeIntervalSince1970: 1_500), completionDate: nil, isCompleted: false, isDeleted: false, isStale: false),
            .init(identifier: "outside", listIdentifier: "work", title: "Outside", recurrenceIdentifier: "", dueDate: Date(timeIntervalSince1970: 3_000), completionDate: nil, isCompleted: false, isDeleted: false, isStale: false),
        ])
        let page = try reader.readReminders(listIdentifiers: ["work"], start: start, end: end, maximum: 2)
        XCTAssertEqual(page.records.map(\.identifier), ["due", "done"])
        XCTAssertFalse(page.truncated)
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
