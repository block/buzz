import EventKit
import Foundation

let battleRhythmCalendarName = "HMAS Supply Battle Rhythm"
private let battleRhythmURLScheme = "command-adviser"
private let battleRhythmURLHost = "battle-rhythm"

struct CalendarProjection: Equatable {
    let externalID: String
    let title: String
    let start: Date
    let end: Date
    let isAllDay: Bool
    let location: String?
    let notes: String?
}

struct StoredCalendarEvent: Equatable {
    let identifier: String
    let externalID: String?
    let title: String
    let start: Date
    let end: Date
    let isAllDay: Bool
    let location: String?
    let notes: String?

    func matches(_ projection: CalendarProjection) -> Bool {
        externalID == projection.externalID
            && title == projection.title
            && start == projection.start
            && end == projection.end
            && isAllDay == projection.isAllDay
            && location == projection.location
            && notes == projection.notes
    }
}

struct CalendarReconcileResult: Equatable {
    let permission: PermissionState
    let calendarIdentifier: String?
    let created: Int
    let updated: Int
    let deleted: Int
    let unchanged: Int
    let error: String?
}

@MainActor
protocol CalendarWritingStore: AnyObject {
    var permission: PermissionState { get }
    func calendarIdentifier(named name: String) -> String?
    func createCalendar(named name: String) throws -> String
    func events(calendarIdentifier: String, start: Date, end: Date) -> [StoredCalendarEvent]
    func save(_ projection: CalendarProjection, calendarIdentifier: String, replacing identifier: String?) throws
    func delete(identifier: String) throws
    func commit() throws
}

@MainActor
final class EventKitWriter {
    private let store: CalendarWritingStore

    init(store: CalendarWritingStore = EventKitCalendarWritingStore()) {
        self.store = store
    }

    func reconcile(
        projections: [CalendarProjection],
        coverageStart: Date,
        coverageEnd: Date
    ) -> CalendarReconcileResult {
        guard store.permission == .authorized else {
            return CalendarReconcileResult(
                permission: store.permission,
                calendarIdentifier: nil,
                created: 0,
                updated: 0,
                deleted: 0,
                unchanged: 0,
                error: "Calendar write permission is required"
            )
        }
        do {
            let calendarIdentifier = try store.calendarIdentifier(named: battleRhythmCalendarName)
                ?? store.createCalendar(named: battleRhythmCalendarName)
            let existing = store.events(
                calendarIdentifier: calendarIdentifier,
                start: coverageStart,
                end: coverageEnd
            )
            let managed = Dictionary(
                uniqueKeysWithValues: existing.compactMap { event in
                    event.externalID.map { ($0, event) }
                }
            )
            let requested = Dictionary(
                uniqueKeysWithValues: projections.map { ($0.externalID, $0) }
            )
            var created = 0
            var updated = 0
            var deleted = 0
            var unchanged = 0
            for projection in projections {
                if let prior = managed[projection.externalID] {
                    if prior.matches(projection) {
                        unchanged += 1
                    } else {
                        try store.save(
                            projection,
                            calendarIdentifier: calendarIdentifier,
                            replacing: prior.identifier
                        )
                        updated += 1
                    }
                } else {
                    try store.save(
                        projection,
                        calendarIdentifier: calendarIdentifier,
                        replacing: nil
                    )
                    created += 1
                }
            }
            for (externalID, event) in managed where requested[externalID] == nil {
                try store.delete(identifier: event.identifier)
                deleted += 1
            }
            try store.commit()
            return CalendarReconcileResult(
                permission: .authorized,
                calendarIdentifier: calendarIdentifier,
                created: created,
                updated: updated,
                deleted: deleted,
                unchanged: unchanged,
                error: nil
            )
        } catch {
            return CalendarReconcileResult(
                permission: .authorized,
                calendarIdentifier: nil,
                created: 0,
                updated: 0,
                deleted: 0,
                unchanged: 0,
                error: error.localizedDescription
            )
        }
    }
}

@MainActor
final class EventKitCalendarWritingStore: CalendarWritingStore {
    private let store = EKEventStore()

    var permission: PermissionState {
        EventKitReader.permissionState(for: EKEventStore.authorizationStatus(for: .event))
    }

    func calendarIdentifier(named name: String) -> String? {
        store.calendars(for: .event)
            .first(where: { $0.title == name })?
            .calendarIdentifier
    }

    func createCalendar(named name: String) throws -> String {
        let calendar = EKCalendar(for: .event, eventStore: store)
        calendar.title = name
        guard let source = store.defaultCalendarForNewEvents?.source
            ?? store.calendars(for: .event).first?.source
        else {
            throw AppleInputFailure.invalidRequest("No writable Apple Calendar source is available")
        }
        calendar.source = source
        try store.saveCalendar(calendar, commit: true)
        return calendar.calendarIdentifier
    }

    func events(calendarIdentifier: String, start: Date, end: Date) -> [StoredCalendarEvent] {
        guard let calendar = store.calendar(withIdentifier: calendarIdentifier) else { return [] }
        let predicate = store.predicateForEvents(withStart: start, end: end, calendars: [calendar])
        return store.events(matching: predicate).map { event in
            StoredCalendarEvent(
                identifier: event.eventIdentifier,
                externalID: Self.externalID(from: event.url),
                title: event.title ?? "",
                start: event.startDate,
                end: event.endDate,
                isAllDay: event.isAllDay,
                location: event.location,
                notes: event.notes
            )
        }
    }

    func save(
        _ projection: CalendarProjection,
        calendarIdentifier: String,
        replacing identifier: String?
    ) throws {
        guard let calendar = store.calendar(withIdentifier: calendarIdentifier) else {
            throw AppleInputFailure.invalidRequest("The dedicated Apple Calendar is unavailable")
        }
        let event = identifier.flatMap(store.event(withIdentifier:)) ?? EKEvent(eventStore: store)
        event.calendar = calendar
        event.title = projection.title
        event.startDate = projection.start
        event.endDate = projection.end
        event.isAllDay = projection.isAllDay
        event.location = projection.location
        event.notes = projection.notes
        event.url = Self.url(for: projection.externalID)
        try store.save(event, span: .thisEvent, commit: false)
    }

    func delete(identifier: String) throws {
        guard let event = store.event(withIdentifier: identifier) else { return }
        try store.remove(event, span: .thisEvent, commit: false)
    }

    func commit() throws {
        try store.commit()
    }

    private static func url(for externalID: String) -> URL? {
        var components = URLComponents()
        components.scheme = battleRhythmURLScheme
        components.host = battleRhythmURLHost
        components.queryItems = [URLQueryItem(name: "id", value: externalID)]
        return components.url
    }

    private static func externalID(from url: URL?) -> String? {
        guard
            let url,
            url.scheme == battleRhythmURLScheme,
            url.host == battleRhythmURLHost
        else { return nil }
        return URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?
            .first(where: { $0.name == "id" })?
            .value
    }
}
