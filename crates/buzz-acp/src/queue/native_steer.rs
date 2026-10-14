//! Retain native follow-ups until both acceptance and turn completion.
use super::*;

pub(super) struct WithheldSteer {
    pub(super) queued: QueuedEvent,
    turn_id: String,
    acknowledged: bool,
    completed: Option<bool>,
}

impl EventQueue {
    /// Steering transfers an admitted event within the shared channel budget.
    /// A capacity eviction must not send an event that is no longer retained.
    pub fn can_native_steer(&self, scope: &SessionScope, event_id: &str) -> bool {
        self.channel_event_total(scope.channel_id()) <= MAX_PENDING_PER_CHANNEL
            && self.queues.get(scope).is_some_and(|events| {
                events
                    .iter()
                    .any(|event| event.event.id.to_hex() == event_id)
            })
    }

    /// Whether this acknowledgement still owns the retained delivery attempt.
    pub fn owns_native_steer(&self, scope: &SessionScope, event_id: &str, turn_id: &str) -> bool {
        self.withheld_native_steer
            .get(scope)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry.turn_id == turn_id && entry.queued.event.id.to_hex() == event_id
                })
            })
    }

    /// Record acceptance without discarding the payload until its turn settles.
    /// Returns false when a failed turn requires ordinary redelivery instead.
    pub fn acknowledge_native_steer(&mut self, scope: &SessionScope, event_id: &str) -> bool {
        let Some(entry) = self
            .withheld_native_steer
            .get_mut(scope)
            .and_then(|entries| {
                entries
                    .iter_mut()
                    .find(|entry| entry.queued.event.id.to_hex() == event_id)
            })
        else {
            return false;
        };
        entry.acknowledged = true;
        match entry.completed {
            Some(true) => self.remove_event(scope, event_id),
            Some(false) => {
                self.release_native_steer(scope, event_id);
                return false;
            }
            None => {}
        }
        true
    }

    /// Settle only this turn's follow-ups. The ack and result may arrive in
    /// either order; entries already settled by an earlier turn stay unchanged.
    pub fn finish_native_steers(&mut self, scope: &SessionScope, turn_id: &str, completed: bool) {
        let Some(entries) = self.withheld_native_steer.get_mut(scope) else {
            return;
        };
        let ready: Vec<_> = entries
            .iter_mut()
            .filter_map(|entry| {
                if entry.turn_id != turn_id || entry.completed.is_some() {
                    return None;
                }
                entry.completed = Some(completed);
                entry.acknowledged.then(|| entry.queued.event.id.to_hex())
            })
            .collect();
        // Release in reverse order because release_native_steer pushes front.
        for event_id in ready.into_iter().rev() {
            if completed {
                self.remove_event(scope, &event_id);
            } else {
                self.release_native_steer(scope, &event_id);
            }
        }
    }

    /// Move a queued event out of `queues[channel_id]` into the side table
    /// to withhold it from `flush_next` while a goose-native steer is in
    /// flight.
    ///
    /// Returns `true` if the event was found and withheld, `false` if the
    /// event id was not present in `queues[channel_id]` (race-safe no-op:
    /// the event may have already been drained, removed, or never queued).
    ///
    /// Must be called synchronously from the mode-gate fork immediately
    /// after `pool.send_steer` returns `Ok(turn_id)` and before any watcher task
    /// is spawned, so the withhold is established before `mark_complete` /
    /// any subsequent `flush_next` tick can run.
    pub fn mark_native_steer_pending<K: IntoScope>(
        &mut self,
        scope: K,
        event_id: &str,
        turn_id: &str,
    ) -> bool {
        let scope = scope.into_scope();
        let Some(q) = self.queues.get_mut(&scope) else {
            return false;
        };
        let Some(pos) = q.iter().position(|qe| qe.event.id.to_hex() == event_id) else {
            return false;
        };
        let qe = q
            .remove(pos)
            .expect("position came from iter so remove must succeed");
        if q.is_empty() {
            self.queues.remove(&scope);
        }
        self.withheld_native_steer
            .entry(scope)
            .or_default()
            .push(WithheldSteer {
                queued: qe,
                turn_id: turn_id.to_owned(),
                acknowledged: false,
                completed: None,
            });
        true
    }

    /// Release a single withheld event back to the front of
    /// `queues[channel_id]`, preserving its original `received_at`.
    ///
    /// Called on `SteerAck::Err(_)` and `SteerAck::PromptCompletedNeutral`
    /// (delivery unknown after prompt completion; restoring queued event
    /// for normal dispatch). Idempotent: a no-op if the event was already
    /// removed or never withheld.
    ///
    /// Push-to-front matches the discipline of `requeue_preserve_timestamps`
    /// at line 453, preserving fairness across channels.
    pub fn release_native_steer<K: IntoScope>(&mut self, scope: K, event_id: &str) {
        let scope = scope.into_scope();
        let Some(entries) = self.withheld_native_steer.get_mut(&scope) else {
            return;
        };
        let Some(pos) = entries
            .iter()
            .position(|qe| qe.queued.event.id.to_hex() == event_id)
        else {
            return;
        };
        let qe = entries.remove(pos);
        if entries.is_empty() {
            self.withheld_native_steer.remove(&scope);
        }
        let channel_id = scope.channel_id();
        // Push to FRONT so original `received_at` keeps the event at the head
        // of the scope's queue. Per-scope cap is enforced below in case
        // a flood of events arrived during the ack window.
        let queue = self.queues.entry(scope.clone()).or_default();
        queue.push_front(qe.queued);
        while queue.len() > MAX_PENDING_PER_SCOPE {
            queue.pop_back();
            tracing::warn!(
                channel_id = %channel_id,
                scope = %scope.telemetry_label(),
                limit = MAX_PENDING_PER_SCOPE,
                "release_native_steer overflow — dropped newest event to enforce cap"
            );
        }
        self.enforce_channel_cap(channel_id);
    }

    /// Drop a specific event by id from both the side table and the main
    /// queue.
    ///
    /// Called once both the steer acknowledgement and turn completion are
    /// successful. Idempotent across both stores.
    pub fn remove_event<K: IntoScope>(&mut self, scope: K, event_id: &str) {
        let scope = scope.into_scope();
        if let Some(entries) = self.withheld_native_steer.get_mut(&scope) {
            entries.retain(|qe| qe.queued.event.id.to_hex() != event_id);
            if entries.is_empty() {
                self.withheld_native_steer.remove(&scope);
            }
        }
        if let Some(q) = self.queues.get_mut(&scope) {
            q.retain(|qe| qe.event.id.to_hex() != event_id);
            if q.is_empty() {
                self.queues.remove(&scope);
            }
        }
    }

    /// Bulk-release every withheld event for `channel_id` back to the queue
    /// front, preserving relative FIFO order.
    ///
    /// Called from the `in_flight_deadline` expiry blocks in
    /// `flush_next` and `has_flushable_work` — if a steer ack never arrives
    /// (read loop hung, watcher never posted), the withheld events would
    /// otherwise be permanently orphaned. Recover, do not log-and-drop: the
    /// events did not complete in the agent, so normal dispatch must
    /// have a chance to deliver them.
    ///
    /// Iterates the stored entries in reverse so per-entry `push_front`
    /// composes to original-FIFO order at the queue front (same discipline
    /// as `requeue_preserve_timestamps` at line 453).
    pub(super) fn recover_withheld_for_expired_scope(&mut self, scope: &SessionScope) {
        let Some(entries) = self.withheld_native_steer.remove(scope) else {
            return;
        };
        let n = entries.len();
        let channel_id = scope.channel_id();
        let queue = self.queues.entry(scope.clone()).or_default();
        for qe in entries.into_iter().rev() {
            queue.push_front(qe.queued);
        }
        while queue.len() > MAX_PENDING_PER_SCOPE {
            queue.pop_back();
            tracing::warn!(
                channel_id = %channel_id,
                scope = %scope.telemetry_label(),
                limit = MAX_PENDING_PER_SCOPE,
                "withheld-steer recovery overflow — dropped newest event to enforce cap"
            );
        }
        self.enforce_channel_cap(channel_id);
        tracing::warn!(
            channel_id = %channel_id,
            scope = %scope.telemetry_label(),
            recovered = n,
            "in-flight expiry recovered withheld steer event(s) — \
             steer ack never arrived; normal dispatch will deliver"
        );
    }
}
