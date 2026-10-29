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
