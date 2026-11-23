use super::*;
use subtle::ConstantTimeEq;

impl PairingSession {
    /// Start source-only code verification once, after capability negotiation.
    /// Returns the code for local display and an encrypted challenge containing
    /// no code or verifier. Repeated calls cannot reset the guess budget.
    pub fn start_desktop_code(&mut self) -> Result<(String, Event), PairingError> {
        self.check_expired()?;
        self.expect_role(Role::Source)?;
        self.expect_state(SessionState::Confirming)?;
        if !self.desktop_code_requested || self.desktop_code.is_some() {
            return Err(PairingError::SasMismatch);
        }
        let code = format!("{:06}", rand::random_range(0..1_000_000u32));
        let challenge = self.build_event(&PairingMessage::DesktopCode {})?;
        self.desktop_code = Some(Zeroizing::new(code.clone()));
        Ok((code, challenge))
    }

    /// Verify a signed submission from the locked peer. Only a separate random
    /// desktop code authorizes release; the QR-derived SAS/transcript cannot.
    /// The response is a source proof on success or a rejection with a remaining
    /// guess budget. Five wrong guesses permanently abort this QR session.
    pub fn handle_target_code(&mut self, event: &Event) -> Result<(Event, bool), PairingError> {
        self.check_expired()?;
        self.expect_role(Role::Source)?;
        self.expect_state(SessionState::Confirming)?;
        self.validate_event_from_peer(event)?;
        let expected = self
            .desktop_code
            .as_ref()
            .ok_or(PairingError::SasMismatch)?;
        let (mut code, request_id) = match self.decrypt_message(event)? {
            PairingMessage::CodeSubmit { code, request_id } => (code, request_id),
            other => return Err(unexpected("code-submit", &other)),
        };
        let correct: bool = code.as_bytes().ct_eq(expected.as_bytes()).into();
        code.zeroize();
        self.code_attempts += 1;
        self.record_event(event);
        if correct {
            self.desktop_code = None;
            return self.confirm_sas().map(|proof| (proof, true));
        }
        let remaining_attempts = 5u8.saturating_sub(self.code_attempts);
        let rejection = self.build_event(&PairingMessage::CodeRejected {
            request_id,
            remaining_attempts,
        })?;
        if remaining_attempts == 0 {
            self.desktop_code = None;
            self.state = SessionState::Aborted;
        }
        Ok((rejection, false))
    }
}

/// Target-only state. The source never learns the pending code locally.
#[derive(Default)]
pub(super) struct TargetDesktopCode {
    pub(super) challenged: bool,
    pub(super) pending_request_id: Option<String>,
    remaining_attempts: Option<u8>,
}

impl PairingSession {
    /// Create a target that requires `desktop-code-v1`, with no SAS fallback.
    ///
    /// The caller must handle the source challenge, submit the user-entered code,
    /// verify the source proof, and call `confirm_target_sas` before importing.
    pub fn new_target_with_desktop_code(qr: &QrPayload) -> Result<(Self, Event), PairingError> {
        Self::new_target_with_confirmation(qr, true)
    }

    /// Validate the locked source's code-entry challenge. It contains no code.
    /// Duplicate or replacement challenges never reset an outstanding attempt.
    pub fn handle_desktop_code(&mut self, event: &Event) -> Result<(), PairingError> {
        self.check_expired()?;
        self.expect_role(Role::Target)?;
        self.expect_state(SessionState::Confirming)?;
        self.validate_event_from_peer(event)?;
        let code = self
            .target_desktop_code
            .as_ref()
            .ok_or(PairingError::SasMismatch)?;
        if code.challenged {
            return Err(PairingError::SasMismatch);
        }
        match self.decrypt_message(event)? {
            PairingMessage::DesktopCode {} => {}
            other => return Err(unexpected("desktop-code", &other)),
        }
        if let Some(code) = &mut self.target_desktop_code {
            code.challenged = true;
        }
        self.record_event(event);
        Ok(())
    }

    /// Encrypt one six-ASCII-digit user submission with a fresh request ID.
    ///
    /// Only one request may be outstanding. A correlated rejection allows the
    /// next submission; callers must not retry a pending guess with a new ID.
    pub fn submit_desktop_code(&mut self, code: Zeroizing<String>) -> Result<Event, PairingError> {
        self.check_expired()?;
        self.expect_role(Role::Target)?;
        self.expect_state(SessionState::Confirming)?;
        let target = self
            .target_desktop_code
            .as_ref()
            .ok_or(PairingError::SasMismatch)?;
        if !target.challenged
            || target.pending_request_id.is_some()
            || code.len() != 6
            || !code.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(PairingError::SasMismatch);
        }
        let request_id = hex::encode(rand::random::<[u8; 16]>());
        let mut message = PairingMessage::CodeSubmit {
            code: (*code).clone(),
            request_id: request_id.clone(),
        };
        let result = self.build_event(&message);
        if let PairingMessage::CodeSubmit { code, .. } = &mut message {
            code.zeroize();
        }
        let event = result?;
        if let Some(target) = &mut self.target_desktop_code {
            target.pending_request_id = Some(request_id);
        }
        Ok(event)
    }

    /// Validate a rejection for the pending request and return guesses remaining.
    ///
    /// Unrelated, stale, duplicate and non-decreasing budgets leave the pending
    /// request unchanged. Zero remaining guesses permanently aborts the session.
    pub fn handle_code_rejected(&mut self, event: &Event) -> Result<u8, PairingError> {
        self.check_expired()?;
        self.expect_role(Role::Target)?;
        self.expect_state(SessionState::Confirming)?;
        self.validate_event_from_peer(event)?;
        let (request_id, remaining_attempts) = match self.decrypt_message(event)? {
            PairingMessage::CodeRejected {
                request_id,
                remaining_attempts,
            } => (request_id, remaining_attempts),
            other => return Err(unexpected("code-rejected", &other)),
        };
        let target = self
            .target_desktop_code
            .as_ref()
            .ok_or(PairingError::SasMismatch)?;
        if target.pending_request_id.as_deref() != Some(request_id.as_str())
            || remaining_attempts >= target.remaining_attempts.unwrap_or(5)
        {
            return Err(PairingError::SasMismatch);
        }
        if let Some(target) = &mut self.target_desktop_code {
            target.pending_request_id = None;
            target.remaining_attempts = Some(remaining_attempts);
        }
        self.record_event(event);
        if remaining_attempts == 0 {
            self.state = SessionState::Aborted;
        }
        Ok(remaining_attempts)
    }
}
