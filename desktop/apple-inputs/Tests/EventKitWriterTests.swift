import XCTest

@MainActor
final class EventKitWriterTests: XCTestCase {
    func testCreatesDedicatedCalendarOnceAndUpdatesStableIDsWithoutDuplicates() {
        let store = FixtureCalendarStore()
        let writer = EventKitWriter(store: store)
        let initial = projection(id: "battle-rhythm:brief", title: "Navigation brief")

        let first = writer.reconcile(
            projections: [initial],
            coverageStart: .distantPast,
            coverageEnd: .distantFuture
        )
        let second = writer.reconcile(
            projections: [projection(id: initial.externalID, title: "Updated navigation brief")],
            coverageStart: .distantPast,
            coverageEnd: .distantFuture
        )

        XCTAssertEqual(first.created, 1)
        XCTAssertEqual(second.updated, 1)
        XCTAssertEqual(store.calendarCreateCount, 1)
        XCTAssertEqual(store.events.count, 1)
        XCTAssertEqual(store.events[0].title, "Updated navigation brief")
    }

    func testDeletesOnlyAbsentManagedEventsAndLeavesUntaggedEventsUntouched() {
        let store = FixtureCalendarStore()
        store.calendarID = "calendar"
        store.events = [
            stored(id: "managed", externalID: "battle-rhythm:old", title: "Old"),
            stored(id: "personal", externalID: nil, title: "Personal"),
        ]

        let result = EventKitWriter(store: store).reconcile(
            projections: [],
            coverageStart: .distantPast,
            coverageEnd: .distantFuture
        )

        XCTAssertEqual(result.deleted, 1)
        XCTAssertEqual(store.events.map(\.identifier), ["personal"])
    }

    func testReconciliationRestoresAuthoritativeFieldsAfterAppleSideEdit() {
        let store = FixtureCalendarStore()
        store.calendarID = "calendar"
        store.events = [
            stored(
                id: "managed",
                externalID: "battle-rhythm:sailing",
                title: "Apple-side edit"
            ),
        ]
        let authoritative = projection(
            id: "battle-rhythm:sailing",
            title: "Sailing stations"
        )

        let result = EventKitWriter(store: store).reconcile(
            projections: [authoritative],
            coverageStart: .distantPast,
            coverageEnd: .distantFuture
        )

        XCTAssertEqual(result.updated, 1)
        XCTAssertEqual(store.events[0].title, "Sailing stations")
    }

    func testPermissionDenialReturnsStatusWithoutMutatingInputOrStore() {
        let store = FixtureCalendarStore(permission: .denied)
        let input = [projection(id: "battle-rhythm:brief", title: "Brief")]

        let result = EventKitWriter(store: store).reconcile(
            projections: input,
            coverageStart: .distantPast,
            coverageEnd: .distantFuture
        )

        XCTAssertEqual(result.permission, .denied)
        XCTAssertEqual(result.created, 0)
        XCTAssertEqual(store.events.count, 0)
        XCTAssertEqual(input[0].title, "Brief")
    }

    private func projection(id: String, title: String) -> CalendarProjection {
        CalendarProjection(
            externalID: id,
            title: title,
            start: Date(timeIntervalSince1970: 1_800_000_000),
            end: Date(timeIntervalSince1970: 1_800_003_600),
            isAllDay: false,
            location: "Bridge",
            notes: "Command Adviser"
        )
    }

    private func stored(
        id: String,
        externalID: String?,
        title: String
    ) -> StoredCalendarEvent {
        StoredCalendarEvent(
            identifier: id,
            externalID: externalID,
            title: title,
            start: Date(timeIntervalSince1970: 1_800_000_000),
            end: Date(timeIntervalSince1970: 1_800_003_600),
            isAllDay: false,
            location: "Bridge",
            notes: "Command Adviser"
        )
    }
}

@MainActor
private final class FixtureCalendarStore: CalendarWritingStore {
    let permission: PermissionState
    var calendarID: String?
    var calendarCreateCount = 0
    var events: [StoredCalendarEvent] = []
    private var nextID = 1

    init(permission: PermissionState = .authorized) {
        self.permission = permission
    }

    func calendarIdentifier(named _: String) -> String? {
        calendarID
    }

    func createCalendar(named _: String) throws -> String {
        calendarCreateCount += 1
        calendarID = "calendar"
        return "calendar"
    }

    func events(calendarIdentifier _: String, start _: Date, end _: Date) -> [StoredCalendarEvent] {
        events
    }

    func save(
        _ projection: CalendarProjection,
        calendarIdentifier _: String,
        replacing identifier: String?
    ) throws {
        let eventID = identifier ?? "event-\(nextID)"
        nextID += 1
        let stored = StoredCalendarEvent(
            identifier: eventID,
            externalID: projection.externalID,
            title: projection.title,
            start: projection.start,
            end: projection.end,
            isAllDay: projection.isAllDay,
            location: projection.location,
            notes: projection.notes
        )
        if let index = events.firstIndex(where: { $0.identifier == eventID }) {
            events[index] = stored
        } else {
            events.append(stored)
        }
    }

    func delete(identifier: String) throws {
        events.removeAll(where: { $0.identifier == identifier })
    }

    func commit() throws {}
}
