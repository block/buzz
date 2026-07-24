import EventKit
import Foundation

struct Page<Value> { let records: [Value]; let truncated: Bool }
struct CalendarRecord: Equatable {
    let identifier, calendarIdentifier, title, recurrenceIdentifier: String
    let start, end: Date
    let isRecurring, isDeleted, isStale: Bool
    func output() -> AppleInputRecord { .init(fields: [
        "identifier": identifier, "calendar_identifier": calendarIdentifier, "title": title,
        "start": ISO8601DateFormatter().string(from: start), "end": ISO8601DateFormatter().string(from: end),
        "is_recurring": isRecurring.description, "recurrence_identifier": recurrenceIdentifier,
        "is_deleted": isDeleted.description, "is_stale": isStale.description,
    ]) }
}
struct ReminderRecord: Equatable {
    let identifier, listIdentifier, title, recurrenceIdentifier: String
    let isCompleted, isDeleted, isStale: Bool
    func output() -> AppleInputRecord { .init(fields: [
        "identifier": identifier, "list_identifier": listIdentifier, "title": title,
        "is_completed": isCompleted.description, "recurrence_identifier": recurrenceIdentifier,
        "is_deleted": isDeleted.description, "is_stale": isStale.description,
    ]) }
}

final class EventKitReader: @unchecked Sendable {
    private let store: EKEventStore?
    private let calendarFixture: [CalendarRecord]
    private let reminderFixture: [ReminderRecord]
    private let fixturePermissions: (calendar: PermissionState, reminders: PermissionState)?
    private(set) var requestCount = 0
    init() { store = EKEventStore(); calendarFixture = []; reminderFixture = []; fixturePermissions = nil }
    private init(calendarRecords: [CalendarRecord], reminderRecords: [ReminderRecord], permissions: (PermissionState, PermissionState)) {
        store = nil; calendarFixture = calendarRecords; reminderFixture = reminderRecords; fixturePermissions = (permissions.0, permissions.1)
    }
    static func fixture(calendarRecords: [CalendarRecord] = [], reminderRecords: [ReminderRecord] = [], calendarPermission: PermissionState = .authorized, remindersPermission: PermissionState = .authorized) -> EventKitReader {
        .init(calendarRecords: calendarRecords, reminderRecords: reminderRecords, permissions: (calendarPermission, remindersPermission))
    }
    static func permissionState(for status: EKAuthorizationStatus) -> PermissionState {
        switch status {
        case .notDetermined: .notDetermined
        case .restricted: .restricted
        case .denied, .writeOnly: .denied
        case .fullAccess: .authorized
        @unknown default: .unavailable
        }
    }
    func permissionStatus(source: PermissionSource) -> PermissionState {
        if let fixturePermissions { return source == .calendar ? fixturePermissions.calendar : fixturePermissions.reminders }
        switch source {
        case .calendar: return Self.permissionState(for: EKEventStore.authorizationStatus(for: .event))
        case .reminders: return Self.permissionState(for: EKEventStore.authorizationStatus(for: .reminder))
        default: return .unavailable
        }
    }
    func permissionStatus() -> (calendar: PermissionState, reminders: PermissionState) {
        (permissionStatus(source: .calendar), permissionStatus(source: .reminders))
    }
    func requestPermission(source: PermissionSource) async -> PermissionState {
        guard let store else { return permissionStatus(source: source) }
        requestCount += 1
        do {
            switch source {
            case .calendar: _ = try await store.requestFullAccessToEvents()
            case .reminders: _ = try await store.requestFullAccessToReminders()
            default: return .unavailable
            }
            return permissionStatus(source: source)
        } catch { return permissionStatus(source: source) }
    }
    func readCalendar(calendarIdentifiers: [String], start: Date, end: Date, maximum: Int) throws -> Page<CalendarRecord> {
        guard start < end, end.timeIntervalSince(start) <= ProtocolLimits.maximumWindow else { throw AppleInputFailure.invalidRequest("invalid calendar window") }
        let source: [CalendarRecord]
        if store == nil {
            source = calendarFixture.filter { calendarIdentifiers.contains($0.calendarIdentifier) && $0.start < end && $0.end > start }
        } else {
            guard permissionStatus(source: .calendar) == .authorized, let store else { return .init(records: [], truncated: false) }
            let calendars = store.calendars(for: .event).filter { calendarIdentifiers.contains($0.calendarIdentifier) }
            source = store.events(matching: store.predicateForEvents(withStart: start, end: end, calendars: calendars)).prefix(maximum + 1).map { event in
                .init(identifier: event.eventIdentifier, calendarIdentifier: event.calendar.calendarIdentifier,
                      title: String((event.title ?? "").prefix(512)), recurrenceIdentifier: event.calendarItemExternalIdentifier,
                      start: event.startDate, end: event.endDate, isRecurring: event.hasRecurrenceRules,
                      isDeleted: false, isStale: false)
            }
        }
        return .init(records: Array(source.prefix(maximum)), truncated: source.count > maximum)
    }
    func readReminders(listIdentifiers: [String], maximum: Int) throws -> Page<ReminderRecord> {
        if store == nil {
            let selected = reminderFixture.filter { listIdentifiers.contains($0.listIdentifier) }
            return .init(records: Array(selected.prefix(maximum)), truncated: selected.count > maximum)
        }
        guard permissionStatus(source: .reminders) == .authorized, let store else { return .init(records: [], truncated: false) }
        let lists = store.calendars(for: .reminder).filter { listIdentifiers.contains($0.calendarIdentifier) }
        let semaphore = DispatchSemaphore(value: 0)
        let lock = NSLock()
        var fetched: [EKReminder] = []
        let token = store.fetchReminders(matching: store.predicateForReminders(in: lists)) { reminders in
            lock.lock(); fetched = reminders ?? []; lock.unlock(); semaphore.signal()
        }
        guard semaphore.wait(timeout: .now() + 5) == .success else {
            store.cancelFetchRequest(token)
            throw AppleInputFailure.invalidRequest("reminder query timed out")
        }
        lock.lock(); let snapshot = fetched; lock.unlock()
        let values = snapshot.prefix(maximum + 1).map { reminder in
            ReminderRecord(identifier: reminder.calendarItemIdentifier, listIdentifier: reminder.calendar.calendarIdentifier,
                           title: String((reminder.title ?? "").prefix(512)), recurrenceIdentifier: reminder.calendarItemExternalIdentifier,
                           isCompleted: reminder.isCompleted, isDeleted: false, isStale: false)
        }
        return .init(records: Array(values.prefix(maximum)), truncated: values.count > maximum)
    }
}
