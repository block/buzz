import EventKit
import Foundation

struct CalendarRecord: Equatable {
    let identifier: String; let calendarIdentifier: String; let title: String; let start: Date; let end: Date; let isRecurring: Bool
    func output() -> AppleInputRecord { .init(fields: ["identifier": identifier, "calendar_identifier": calendarIdentifier, "title": title, "start": ISO8601DateFormatter().string(from: start), "end": ISO8601DateFormatter().string(from: end), "is_recurring": isRecurring.description]) }
}
struct ReminderRecord: Equatable {
    let identifier: String; let listIdentifier: String; let title: String; let isCompleted: Bool
    func output() -> AppleInputRecord { .init(fields: ["identifier": identifier, "list_identifier": listIdentifier, "title": title, "is_completed": isCompleted.description]) }
}

final class EventKitReader: @unchecked Sendable {
    private let store: EKEventStore?
    private let calendarFixture: [CalendarRecord]
    private let reminderFixture: [ReminderRecord]
    private let fixturePermissions: (calendar: PermissionState, reminders: PermissionState)?
    private(set) var requestCount = 0

    init() { store = EKEventStore(); calendarFixture = []; reminderFixture = []; fixturePermissions = nil }
    private init(calendarRecords: [CalendarRecord], reminderRecords: [ReminderRecord], permissions: (PermissionState, PermissionState)) { store = nil; calendarFixture = calendarRecords; reminderFixture = reminderRecords; fixturePermissions = (permissions.0, permissions.1) }
    static func fixture(calendarRecords: [CalendarRecord] = [], reminderRecords: [ReminderRecord] = [], calendarPermission: PermissionState = .authorized, remindersPermission: PermissionState = .authorized) -> EventKitReader { .init(calendarRecords: calendarRecords, reminderRecords: reminderRecords, permissions: (calendarPermission, remindersPermission)) }

    func permissionStatus() -> (calendar: PermissionState, reminders: PermissionState) {
        if let fixturePermissions { return fixturePermissions }
        return (state(for: .event), state(for: .reminder))
    }

    func requestPermission() async -> (calendar: PermissionState, reminders: PermissionState) {
        guard let store else { return permissionStatus() }
        requestCount += 1
        do { if #available(macOS 14.0, *) { _ = try await store.requestFullAccessToEvents(); _ = try await store.requestFullAccessToReminders() } else { _ = try await store.requestAccess(to: .event); _ = try await store.requestAccess(to: .reminder) } } catch { }
        return permissionStatus()
    }

    func readCalendar(calendarIdentifiers: [String], start: Date, end: Date, maximum: Int) throws -> [CalendarRecord] {
        let limit = try bounded(maximum, maximum: 100)
        guard start < end else { throw AppleInputFailure.invalidRequest("calendar range must be ascending") }
        if store == nil { return Array(calendarFixture.filter { calendarIdentifiers.contains($0.calendarIdentifier) && $0.start < end && $0.end > start }.prefix(limit)) }
        guard permissionStatus().calendar == .authorized, let store else { return [] }
        let calendars = store.calendars(for: .event).filter { calendarIdentifiers.contains($0.calendarIdentifier) }
        return store.events(matching: store.predicateForEvents(withStart: start, end: end, calendars: calendars)).prefix(limit).map { event in
            CalendarRecord(identifier: event.eventIdentifier, calendarIdentifier: event.calendar.calendarIdentifier, title: event.title ?? "", start: event.startDate, end: event.endDate, isRecurring: event.hasRecurrenceRules)
        }
    }

    func readReminders(listIdentifiers: [String], maximum: Int) throws -> [ReminderRecord] {
        let limit = try bounded(maximum, maximum: 100)
        if store == nil { return Array(reminderFixture.filter { listIdentifiers.contains($0.listIdentifier) }.prefix(limit)) }
        guard permissionStatus().reminders == .authorized, let store else { return [] }
        let lists = store.calendars(for: .reminder).filter { listIdentifiers.contains($0.calendarIdentifier) }
        let semaphore = DispatchSemaphore(value: 0)
        var result: [EKReminder] = []
        store.fetchReminders(matching: store.predicateForReminders(in: lists)) { reminders in result = reminders ?? []; semaphore.signal() }
        guard semaphore.wait(timeout: .now() + 5) == .success else { throw AppleInputFailure.invalidRequest("reminder query timed out") }
        return result.prefix(limit).map { reminder in ReminderRecord(identifier: reminder.calendarItemIdentifier, listIdentifier: reminder.calendar.calendarIdentifier, title: reminder.title ?? "", isCompleted: reminder.isCompleted) }
    }

    private func state(for type: EKEntityType) -> PermissionState { switch EKEventStore.authorizationStatus(for: type) { case .notDetermined: .notDetermined; case .restricted: .restricted; case .denied: .denied; case .fullAccess, .writeOnly: .authorized; @unknown default: .unavailable } }
}
