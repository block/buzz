/// Performs the ordered state transition when the configured push gateway changes.
public enum BuzzPushGatewayStateReset {
  /// Restores current-gateway state and journals retired state before replacing active state.
  public static func run(
    gatewayOrigin: String,
    records: [BuzzPushEndpointGrantRecord],
    pendingEnrollments: [BuzzPushPendingEnrollmentRecord],
    cleanupStates: [BuzzPushGatewayCleanupState],
    saveCleanupState: (BuzzPushGatewayCleanupState) throws -> Void,
    removeCleanupState: (String) throws -> Void,
    replaceRecords: ([BuzzPushEndpointGrantRecord]) throws -> Void,
    replacePendingEnrollments: ([BuzzPushPendingEnrollmentRecord]) throws -> Void
  ) throws {
    var nextRecords = records
    var nextPending = pendingEnrollments
    let restoredState = cleanupStates.first { $0.gatewayOrigin == gatewayOrigin }
    let revocationPendingHandles = Set(
      restoredState?.revocationPendingInstallationHandles ?? []
    )
    if let restoredState {
      for record in restoredState.grants
      where record.gatewayInstallationHandle.map(revocationPendingHandles.contains) != true
        && !nextRecords.contains(where: {
        $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
          && $0.appProfile == record.appProfile
      }) {
        nextRecords.append(record)
      }
      for pending in restoredState.pendingEnrollments
      where pending.gatewayInstallationHandle.map(revocationPendingHandles.contains) != true
        && !nextPending.contains(where: {
        $0.gatewayOrigin == pending.gatewayOrigin && $0.relayOrigin == pending.relayOrigin
          && $0.appProfile == pending.appProfile
      }) {
        nextPending.append(pending)
      }
    }

    let staleRecords = nextRecords.filter { $0.gatewayOrigin != gatewayOrigin }
    let stalePending = nextPending.filter { $0.gatewayOrigin != gatewayOrigin }
    let staleOrigins = Set(staleRecords.map(\.gatewayOrigin) + stalePending.map(\.gatewayOrigin))

    for origin in staleOrigins.sorted() {
      var state = cleanupStates.first { $0.gatewayOrigin == origin }
        ?? BuzzPushGatewayCleanupState(
          gatewayOrigin: origin,
          grants: [],
          pendingEnrollments: []
        )
      for record in staleRecords where record.gatewayOrigin == origin {
        state.grants.removeAll {
          $0.relayOrigin == record.relayOrigin && $0.appProfile == record.appProfile
        }
        state.grants.append(record)
      }
      for pending in stalePending where pending.gatewayOrigin == origin {
        state.pendingEnrollments.removeAll {
          $0.relayOrigin == pending.relayOrigin && $0.appProfile == pending.appProfile
        }
        state.pendingEnrollments.append(pending)
      }
      try saveCleanupState(state)
    }

    if !staleRecords.isEmpty || nextRecords.count != records.count {
      try replaceRecords(nextRecords.filter { $0.gatewayOrigin == gatewayOrigin })
    }
    if !stalePending.isEmpty || nextPending.count != pendingEnrollments.count {
      try replacePendingEnrollments(
        nextPending.filter { $0.gatewayOrigin == gatewayOrigin }
      )
    }
    if let restoredState {
      let retainedGrants = restoredState.grants.filter {
        $0.gatewayInstallationHandle.map(revocationPendingHandles.contains) == true
      }
      let retainedPending = restoredState.pendingEnrollments.filter {
        $0.gatewayInstallationHandle.map(revocationPendingHandles.contains) == true
      }
      if retainedGrants.isEmpty && retainedPending.isEmpty {
        try removeCleanupState(gatewayOrigin)
      } else {
        try saveCleanupState(
          BuzzPushGatewayCleanupState(
            gatewayOrigin: gatewayOrigin,
            grants: retainedGrants,
            pendingEnrollments: retainedPending,
            revocationPendingInstallationHandles: Array(revocationPendingHandles).sorted()
          )
        )
      }
    }
  }
}
