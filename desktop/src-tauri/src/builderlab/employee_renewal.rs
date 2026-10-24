//! Public continuity pins are not authorization: every renewal uses a fresh Square handoff.
use super::{BuilderlabSession, EmployeeBinding};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct EmployeeAccount {
    pub(crate) subject: String,
    pub(crate) org: String,
    pub(crate) workspace: String,
}

impl BuilderlabSession {
    pub(crate) fn employee_renewal_binding(
        &self,
        state: &crate::app_state::AppState,
    ) -> Result<Option<EmployeeBinding>, String> {
        let mut binding = self.2.lock().map_err(|_| "Square session unavailable")?;
        if binding.is_none() {
            return Ok(None);
        }
        let pubkey = state
            .signing_keys()
            .map_err(|_| "Buzz identity unavailable")?
            .public_key()
            .to_hex();
        let relay = crate::relay::relay_ws_url_with_override(state);
        if binding
            .as_ref()
            .is_some_and(|value| value.pubkey != pubkey || value.relay != relay)
        {
            *binding = None;
        }
        Ok(binding.clone())
    }

    pub(super) fn clear_employee_renewal(&self) -> Result<(), String> {
        *self.2.lock().map_err(|_| "Square session unavailable")? = None;
        Ok(())
    }

    pub(crate) fn validate_employee_renewal(&self, next: &EmployeeBinding) -> Result<(), String> {
        let binding = self.2.lock().map_err(|_| "Square session unavailable")?;
        if binding.as_ref().is_some_and(|prior| {
            prior.account != next.account
                || prior.pubkey != next.pubkey
                || prior.relay != next.relay
        }) {
            return Err("Renew with the same Square account and community. To change accounts, sign out of Square first.".to_owned());
        }
        Ok(())
    }

    // Preserve first-login admission recovery without replacing any existing session.
    // A refused candidate has no assertion and cannot authorize relay traffic.
    pub(crate) fn retain_initial_refusal(
        &self,
        login: &super::BuilderlabLogin,
        id: uuid::Uuid,
        candidate: &BuilderlabSession,
    ) -> Result<bool, String> {
        let pending = login.0.lock().map_err(|_| "Square sign-in unavailable")?;
        if pending.as_ref().is_none_or(|value| value.id != id) {
            return Ok(false);
        }
        let mut staged = candidate
            .0
            .lock()
            .map_err(|_| "Square session unavailable")?;
        let mut current = self.0.lock().map_err(|_| "Square session unavailable")?;
        let pin = self.2.lock().map_err(|_| "Square session unavailable")?;
        if current.is_some()
            || pin.is_some()
            || staged.as_ref().is_none_or(|next| {
                next.refusal.is_none()
                    || next.assertion.is_some()
                    || next
                        .employee
                        .as_ref()
                        .is_none_or(|binding| binding.expires_at <= chrono::Utc::now().timestamp())
            })
        {
            return Ok(false);
        }
        *current = staged.take();
        Ok(true)
    }

    pub(crate) fn commit_verified_employee_login(
        &self,
        login: &super::BuilderlabLogin,
        id: uuid::Uuid,
        candidate: &BuilderlabSession,
    ) -> Result<(), String> {
        let mut pending = login.0.lock().map_err(|_| "Square sign-in unavailable")?;
        if pending.as_ref().is_none_or(|value| value.id != id) {
            return Err("Square sign-in canceled".to_owned());
        }
        let mut staged = candidate
            .0
            .lock()
            .map_err(|_| "Square session unavailable")?;
        let next = staged.as_ref().ok_or("Square session changed")?;
        let binding = next
            .employee
            .as_ref()
            .ok_or("Square employee session required")?;
        let now = chrono::Utc::now().timestamp();
        if binding.expires_at <= now
            || next.refusal.is_some()
            || next
                .assertion
                .as_ref()
                .is_none_or(|assertion| assertion.expires_at <= now)
        {
            return Err("Fresh Square relay authorization is required".to_owned());
        }
        // A retry may establish the first continuity pin while this candidate is
        // obtaining admission. Validate, replace, and publish the pin atomically
        // using the same active-session -> pin lock order as retry and sign-out.
        let mut current = self.0.lock().map_err(|_| "Square session unavailable")?;
        let mut pin = self.2.lock().map_err(|_| "Square session unavailable")?;
        if pin.as_ref().is_some_and(|prior| {
            prior.account != binding.account
                || prior.pubkey != binding.pubkey
                || prior.relay != binding.relay
        }) {
            return Err("Renew with the same Square account and community. To change accounts, sign out of Square first.".to_owned());
        }
        let binding = binding.clone();
        if let Some(previous) = current.replace(staged.take().ok_or("Square session changed")?) {
            previous.cancel.cancel();
        }
        *pin = Some(binding);
        *pending = None;
        Ok(())
    }

    pub(crate) async fn refresh_employee_access(
        &self,
        state: &crate::app_state::AppState,
        keys: &nostr::Keys,
        relay: &str,
    ) -> Result<(), String> {
        self.evidence_with_refresh(state, keys, relay, true).await?;
        self.remember_current_employee_binding(state)
    }

    pub(crate) fn remember_current_employee_binding(
        &self,
        state: &crate::app_state::AppState,
    ) -> Result<(), String> {
        let (credential, _) = self
            .request_context(state)?
            .ok_or("Square session changed")?;
        self.remember_employee_binding(&credential)
    }

    pub(crate) fn remember_employee_binding(&self, credential: &str) -> Result<(), String> {
        let stored = self.0.lock().map_err(|_| "Square session unavailable")?;
        let binding = stored
            .as_ref()
            .filter(|stored| {
                stored.credential == credential
                    && stored.assertion.is_some()
                    && stored.refusal.is_none()
            })
            .and_then(|stored| stored.employee.as_ref())
            .ok_or("Square session changed before renewal completed")?;
        *self.2.lock().map_err(|_| "Square session unavailable")? = Some(binding.clone());
        Ok(())
    }
}

#[cfg(test)]
impl EmployeeAccount {
    pub(crate) fn test_account() -> Self {
        Self {
            subject: "employee".to_owned(),
            org: "org".to_owned(),
            workspace: "workspace".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builderlab::{BuilderlabLogin, CachedAssertion};

    #[test]
    fn expiry_requires_fresh_same_account_handoff_and_signout_removes_continuity_pin() {
        let state = crate::app_state::build_app_state();
        let session = BuilderlabSession::default();
        let login = BuilderlabLogin::default();
        let binding = EmployeeBinding {
            account: EmployeeAccount::test_account(),
            pubkey: state.signing_keys().unwrap().public_key().to_hex(),
            relay: crate::relay::relay_ws_url_with_override(&state),
            expires_at: chrono::Utc::now().timestamp() + 240,
        };
        let (id, _) = login.begin_employee_login().unwrap();
        session
            .finish_employee_login(
                &login,
                id,
                "first".to_owned(),
                super::super::BUILDERLAB_API_BASE_URL,
                binding.clone(),
            )
            .unwrap();
        assert!(
            session.remember_employee_binding("first").is_err(),
            "a handoff without relay admission is not a continuity pin"
        );
        let cancel = {
            let mut stored = session.0.lock().unwrap();
            let stored = stored.as_mut().unwrap();
            stored.assertion = Some(CachedAssertion {
                value: "assertion".to_owned(),
                expires_at: binding.expires_at,
            });
            stored.cancel.clone()
        };
        session.remember_current_employee_binding(&state).unwrap();
        for failure in [
            "no_assertion",
            "expired_assertion",
            "denied",
            "wrong_account",
            "canceled",
        ] {
            let candidate = BuilderlabSession::default();
            let (id, _) = login.begin_employee_login().unwrap();
            let mut next = binding.clone();
            if failure == "wrong_account" {
                next.account.subject = "other".to_owned();
            }
            candidate
                .finish_employee_login(
                    &login,
                    id,
                    "candidate".to_owned(),
                    super::super::BUILDERLAB_API_BASE_URL,
                    next,
                )
                .unwrap();
            {
                let mut staged = candidate.0.lock().unwrap();
                let staged = staged.as_mut().unwrap();
                if failure != "no_assertion" {
                    staged.assertion = Some(CachedAssertion {
                        value: "assertion".to_owned(),
                        expires_at: if failure == "expired_assertion" {
                            0
                        } else {
                            binding.expires_at
                        },
                    });
                }
                if failure == "denied" {
                    staged.refusal = Some("authorization_denied".to_owned());
                }
            }
            if failure == "canceled" {
                let _ = login.begin_employee_login().unwrap();
            }
            assert!(
                session
                    .commit_verified_employee_login(&login, id, &candidate)
                    .is_err(),
                "{failure}"
            );
            assert_eq!(session.request_context(&state).unwrap().unwrap().0, "first");
            assert!(
                !cancel.is_cancelled(),
                "a failed candidate must not replace active access"
            );
        }

        session
            .0
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .employee
            .as_mut()
            .unwrap()
            .expires_at = chrono::Utc::now().timestamp() - 1;
        assert!(session.request_context(&state).is_err());
        assert!(
            session.0.lock().unwrap().is_none(),
            "expired credentials must be discarded"
        );
        assert!(cancel.is_cancelled());
        assert!(session.employee_expiry(&state).is_none());
        assert!(!session.employee_can_retry(&state));
        assert!(session.employee_renewal_binding(&state).unwrap().is_some());
        for field in ["subject", "org", "workspace", "relay", "key"] {
            let mut wrong = binding.clone();
            match field {
                "subject" => wrong.account.subject = "other".to_owned(),
                "org" => wrong.account.org = "other".to_owned(),
                "workspace" => wrong.account.workspace = "other".to_owned(),
                "relay" => wrong.relay = "wss://other.example".to_owned(),
                _ => wrong.pubkey = "other".to_owned(),
            }
            let (id, _) = login.begin_employee_login().unwrap();
            assert!(
                session
                    .finish_employee_login(
                        &login,
                        id,
                        "wrong".to_owned(),
                        super::super::BUILDERLAB_API_BASE_URL,
                        wrong
                    )
                    .is_err(),
                "{field}"
            );
            assert!(session.0.lock().unwrap().is_none());
        }
        let (old, _) = login.begin_employee_login().unwrap();
        let (id, _) = login.begin_employee_login().unwrap();
        assert!(session
            .finish_employee_login(
                &login,
                old,
                "canceled".to_owned(),
                super::super::BUILDERLAB_API_BASE_URL,
                binding.clone()
            )
            .is_err());
        let candidate = BuilderlabSession::default();
        candidate
            .finish_employee_login(
                &login,
                id,
                "renewed".to_owned(),
                super::super::BUILDERLAB_API_BASE_URL,
                binding.clone(),
            )
            .unwrap();
        candidate.0.lock().unwrap().as_mut().unwrap().assertion = Some(CachedAssertion {
            value: "fresh assertion".to_owned(),
            expires_at: binding.expires_at,
        });
        session
            .commit_verified_employee_login(&login, id, &candidate)
            .unwrap();
        assert!(candidate.0.lock().unwrap().is_none());
        assert!(login.0.lock().unwrap().is_none());
        assert_eq!(session.employee_expiry(&state), Some(binding.expires_at));
        assert_eq!(
            session.request_context(&state).unwrap().unwrap().0,
            "renewed"
        );
        session.clear().unwrap();
        assert!(session.employee_renewal_binding(&state).unwrap().is_none());
        assert!(session.request_context(&state).unwrap().is_none());
    }
    #[test]
    fn changing_context_clears_an_expired_public_pin_without_restoring_credentials() {
        for change_key in [true, false] {
            let state = crate::app_state::build_app_state();
            let session = BuilderlabSession::default();
            *session.2.lock().unwrap() = Some(EmployeeBinding {
                account: EmployeeAccount::test_account(),
                pubkey: state.signing_keys().unwrap().public_key().to_hex(),
                relay: crate::relay::relay_ws_url_with_override(&state),
                expires_at: 0,
            });
            if change_key {
                *state.keys.lock().unwrap() = nostr::Keys::generate();
            } else {
                *state.relay_url_override.lock().unwrap() = Some("wss://other.example".to_owned());
            }
            assert!(session.request_context(&state).unwrap().is_none());
            assert!(session.2.lock().unwrap().is_none());
        }
    }
    #[test]
    fn first_denied_admission_can_retry_but_cannot_replace_access_completed_in_another_window() {
        let state = crate::app_state::build_app_state();
        let session = BuilderlabSession::default();
        let login = BuilderlabLogin::default();
        let binding = EmployeeBinding {
            account: EmployeeAccount::test_account(),
            pubkey: state.signing_keys().unwrap().public_key().to_hex(),
            relay: crate::relay::relay_ws_url_with_override(&state),
            expires_at: chrono::Utc::now().timestamp() + 240,
        };
        let denied = || {
            let candidate = BuilderlabSession::default();
            let (id, _) = login.begin_employee_login().unwrap();
            candidate
                .finish_employee_login(
                    &login,
                    id,
                    "candidate".to_owned(),
                    super::super::BUILDERLAB_API_BASE_URL,
                    binding.clone(),
                )
                .unwrap();
            candidate
                .refuse_employee_access("candidate", "authorization_denied".to_owned())
                .unwrap();
            (candidate, id)
        };
        let (candidate, id) = denied();
        assert!(session
            .retain_initial_refusal(&login, id, &candidate)
            .unwrap());
        assert!(session.employee_can_retry(&state));
        assert!(session.employee_expiry(&state).is_none());
        assert!(session.employee_renewal_binding(&state).unwrap().is_none());
        // A different-account browser handoff can begin while initial access is
        // still refused. The final commit must honor a pin established by retry
        // after that handoff's initial validation.
        let other = BuilderlabSession::default();
        let (other_id, _) = login.begin_employee_login().unwrap();
        let mut other_binding = binding.clone();
        other_binding.account.subject = "other employee".to_owned();
        session.validate_employee_renewal(&other_binding).unwrap();
        other
            .finish_employee_login(
                &login,
                other_id,
                "other candidate".to_owned(),
                super::super::BUILDERLAB_API_BASE_URL,
                other_binding,
            )
            .unwrap();
        other.0.lock().unwrap().as_mut().unwrap().assertion = Some(CachedAssertion {
            value: "other assertion".to_owned(),
            expires_at: binding.expires_at,
        });
        {
            let mut stored = session.0.lock().unwrap();
            let stored = stored.as_mut().unwrap();
            stored.credential = "accepted".to_owned();
            stored.refusal = None;
            stored.assertion = Some(CachedAssertion {
                value: "assertion".to_owned(),
                expires_at: binding.expires_at,
            });
        }
        session.remember_current_employee_binding(&state).unwrap();
        assert!(session
            .commit_verified_employee_login(&login, other_id, &other)
            .is_err());
        assert!(other.0.lock().unwrap().is_some());
        assert_eq!(
            session.request_context(&state).unwrap().unwrap().0,
            "accepted"
        );
        assert!(session.2.lock().unwrap().as_ref().unwrap().account == binding.account);
        let (candidate, id) = denied();
        assert!(!session
            .retain_initial_refusal(&login, id, &candidate)
            .unwrap());
        assert_eq!(
            session.request_context(&state).unwrap().unwrap().0,
            "accepted"
        );
    }
}
