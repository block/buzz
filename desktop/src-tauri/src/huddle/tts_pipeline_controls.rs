use super::*;

impl TtsPipeline {
    /// Queue `text` for TTS synthesis and playback.
    ///
    /// Non-blocking. Returns `Err` if the queue is full (bounded at
    /// `TEXT_QUEUE_DEPTH`) — caller may log and discard.
    pub fn speak(&self, text: String) -> Result<(), String> {
        self.text_tx
            .try_send(QueuedText {
                generation: self.voice_generation.load(Ordering::Acquire),
                route_id: 0,
                speaker_pubkey: None,
                speaker_generation: 0,
                voice_reference: None,
                text,
            })
            .map_err(|e| {
                eprintln!("buzz-desktop: TTS queue saturated, dropping message: {e}");
                format!("TTS queue full, dropping: {e}")
            })
    }

    /// Clone the bounded queue sender so callers can apply backpressure without
    /// holding the huddle mutex. Disabling TTS drops the receiver and unblocks
    /// any waiting sender while the shared cancellation flag stops playback.
    pub(crate) fn text_sender(&self) -> TtsTextSender {
        TtsTextSender {
            text_tx: self.text_tx.clone(),
            generation: self.voice_generation.load(Ordering::Acquire),
            speaker_generations: Arc::clone(&self.speaker_generations),
        }
    }

    /// Invalidate speech queued for one agent and cancel the player only when
    /// that same agent currently owns it.
    pub(crate) fn cancel_speaker(&self, speaker_pubkey: &str) {
        request_speaker_cancel(
            &self.speaker_generations,
            &self.active_speaker,
            &self.speaker_cancel,
            speaker_pubkey,
        );
    }

    /// Cancel exactly the speaker utterance currently owning playback.
    ///
    /// The speaker generation is advanced while ownership is locked, so a
    /// stale Stop click cannot cancel a later utterance that starts after the
    /// observed one drains.
    pub(crate) fn cancel_active_speaker(&self, expected_speaker_pubkey: &str) -> bool {
        request_active_speaker_cancel(
            &self.speaker_generations,
            &self.active_speaker,
            &self.speaker_cancel,
            &self.playback_probe,
            expected_speaker_pubkey,
        )
    }

    /// Select a bundled Pocket voice for subsequent speech.
    ///
    /// Current playback and queued text are cancelled immediately so content
    /// cannot continue in the old voice. The worker keeps its warmed inference
    /// engine and reloads only the reference style before the next utterance.
    pub fn select_voice(&self, voice: &str) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let acknowledged = begin_voice_change(
            &self.voice,
            &self.voice_generation,
            &self.voice_cancel,
            &self.voice_change_ack,
            voice,
        );
        if acknowledged.is_some() {
            eprintln!("buzz-desktop: tts stage=cancellation reason=voice_switch route_id=0");
        }
        acknowledged
    }

    /// Reconcile the voice of a pipeline that has not been published yet.
    ///
    /// No caller can enqueue text before publication, so raising the shared
    /// cancellation flag here would create a race that could discard the first
    /// message queued immediately after installation.
    pub(crate) fn select_voice_before_publish(&self, voice: &str) {
        *self.voice.lock().unwrap_or_else(|error| error.into_inner()) = voice.to_string();
    }

    /// Signal the worker thread to stop.
    pub fn shutdown(&self) {
        eprintln!("buzz-desktop: tts stage=cancellation reason=shutdown route_id=0");
        self.shutdown.store(true, Ordering::Release);
    }

    /// Returns `true` if the worker thread has exited (init failure, crash, or normal exit).
    /// Used by hot-start to detect dead pipelines and clear them for retry.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|h| h.is_finished())
    }
}
