//! Thin Rust wrapper around Buzz's macOS `sirittsd` Objective-C bridge.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiriVoice {
    pub name: String,
    pub language: String,
    pub identifier: String,
    pub size_bytes: i64,
    pub availability: SiriVoiceAvailability,
    pub version: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiriVoiceAvailability {
    Installed,
    Available,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_float, c_void, CStr, CString};

    use serde::Deserialize;

    use super::{SiriVoice, SiriVoiceAvailability};

    pub type AudioCallback = unsafe extern "C" fn(*mut c_void, *const c_float, u32, f64);
    pub type CompletionCallback = unsafe extern "C" fn(*mut c_void, *const c_char);

    unsafe extern "C" {
        fn buzz_siri_session_create(
            context: *mut c_void,
            audio_callback: AudioCallback,
            completion_callback: CompletionCallback,
        ) -> *mut c_void;
        fn buzz_siri_session_synthesize(
            session: *mut c_void,
            text: *const c_char,
            language: *const c_char,
            voice: *const c_char,
            rate: c_float,
        );
        fn buzz_siri_session_cancel(session: *mut c_void);
        fn buzz_siri_session_release(session: *mut c_void);
        fn buzz_siri_discover_voices_json(prefix: *const c_char) -> *mut c_char;
        fn buzz_siri_downloaded_voices_json(
            language: *const c_char,
            voice: *const c_char,
        ) -> *mut c_char;
        fn buzz_siri_trigger_voice_download(language: *const c_char, voice: *const c_char) -> i32;
        fn buzz_siri_free_string(value: *mut c_char);
    }

    #[derive(Deserialize)]
    struct CatalogVoice {
        name: String,
        language: String,
        identifier: String,
        size_bytes: i64,
    }

    #[derive(Deserialize)]
    struct DownloadedVoice {
        name: String,
        language: String,
        version: Option<i64>,
    }

    fn with_bridge_json<T: for<'de> Deserialize<'de>>(pointer: *mut c_char) -> Result<T, String> {
        if pointer.is_null() {
            return Err("Siri TTS returned no response".into());
        }
        // SAFETY: bridge strings are NUL-terminated malloc allocations and
        // remain owned by us until buzz_siri_free_string is called.
        let bytes = unsafe {
            let bytes = CStr::from_ptr(pointer).to_bytes().to_vec();
            buzz_siri_free_string(pointer);
            bytes
        };
        let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        serde_json::from_str(text).map_err(|error| error.to_string())
    }

    pub fn list_voices(language_prefix: &str) -> Result<Vec<SiriVoice>, String> {
        let prefix = CString::new(language_prefix).map_err(|error| error.to_string())?;
        // SAFETY: prefix is a valid C string for the duration of the call.
        let candidates: Vec<CatalogVoice> =
            with_bridge_json(unsafe { buzz_siri_discover_voices_json(prefix.as_ptr()) })?;
        std::thread::scope(|scope| {
            let handles: Vec<_> = candidates
                .into_iter()
                .map(|voice| {
                    scope.spawn(move || {
                        let installed = validate_voice(&voice.name, &voice.language).ok().flatten();
                        SiriVoice {
                            name: voice.name,
                            language: voice.language,
                            identifier: voice.identifier,
                            size_bytes: voice.size_bytes,
                            availability: if installed.is_some() {
                                SiriVoiceAvailability::Installed
                            } else {
                                SiriVoiceAvailability::Available
                            },
                            version: installed.and_then(|match_| match_.version),
                        }
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "Siri voice validation thread panicked".to_string())
                })
                .collect()
        })
    }

    fn validate_voice(name: &str, language: &str) -> Result<Option<DownloadedVoice>, String> {
        let name = CString::new(name).map_err(|error| error.to_string())?;
        let language = CString::new(language).map_err(|error| error.to_string())?;
        // SAFETY: both values remain alive for the synchronous bridge call.
        let voices: Vec<DownloadedVoice> = with_bridge_json(unsafe {
            buzz_siri_downloaded_voices_json(language.as_ptr(), name.as_ptr())
        })?;
        Ok(voices.into_iter().find(|voice| {
            voice
                .name
                .eq_ignore_ascii_case(name.to_str().unwrap_or_default())
                && normalize_language(&voice.language)
                    == normalize_language(language.to_str().unwrap_or_default())
        }))
    }

    pub fn is_voice_installed(name: &str, language: &str) -> Result<bool, String> {
        Ok(validate_voice(name, language)?.is_some())
    }

    pub fn trigger_download(name: &str, language: &str) -> Result<(), String> {
        let name = CString::new(name).map_err(|error| error.to_string())?;
        let language = CString::new(language).map_err(|error| error.to_string())?;
        // SAFETY: both values remain alive for the synchronous bridge call.
        let result = unsafe { buzz_siri_trigger_voice_download(language.as_ptr(), name.as_ptr()) };
        if result == 1 {
            Ok(())
        } else {
            Err("macOS could not start the Siri voice download".into())
        }
    }

    fn normalize_language(language: &str) -> String {
        language.replace('_', "-").to_lowercase()
    }

    pub struct Session(*mut c_void);

    impl Session {
        pub fn new(
            context: *mut c_void,
            audio_callback: AudioCallback,
            completion_callback: CompletionCallback,
        ) -> Result<Self, String> {
            // SAFETY: callbacks and context are kept alive by the caller until
            // synthesis completes and this Session is dropped.
            let pointer =
                unsafe { buzz_siri_session_create(context, audio_callback, completion_callback) };
            if pointer.is_null() {
                Err("Could not create Siri TTS session".into())
            } else {
                Ok(Self(pointer))
            }
        }

        pub fn synthesize(
            &self,
            text: &str,
            language: &str,
            voice: &str,
            rate: f32,
        ) -> Result<(), String> {
            let text = CString::new(text).map_err(|error| error.to_string())?;
            let language = CString::new(language).map_err(|error| error.to_string())?;
            let voice = CString::new(voice).map_err(|error| error.to_string())?;
            // SAFETY: all C strings live for the synchronous request setup.
            unsafe {
                buzz_siri_session_synthesize(
                    self.0,
                    text.as_ptr(),
                    language.as_ptr(),
                    voice.as_ptr(),
                    rate,
                );
            }
            Ok(())
        }

        pub fn cancel(&self) {
            // SAFETY: self owns a live retained bridge session.
            unsafe { buzz_siri_session_cancel(self.0) };
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: the retained pointer is released exactly once here.
            unsafe { buzz_siri_session_release(self.0) };
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod playback {
    use std::{
        ffi::{c_char, c_void, CStr},
        num::NonZero,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver},
            Arc, Condvar, Mutex, MutexGuard, PoisonError,
        },
        time::Duration,
    };

    use super::Session;
    use crate::huddle::{
        audio_output::open_output_sink_by_name, preprocessing::preprocess_for_tts,
    };

    const RECV_TIMEOUT: Duration = Duration::from_millis(100);
    const MONITOR_TICK: Duration = Duration::from_millis(10);

    struct CallbackContext {
        player: Arc<rodio::Player>,
        tts_active: Arc<AtomicBool>,
        completion: Mutex<Option<Result<(), String>>>,
        completion_ready: Condvar,
    }

    unsafe extern "C" fn audio_callback(
        context: *mut c_void,
        samples: *const f32,
        frame_count: u32,
        sample_rate: f64,
    ) {
        if context.is_null() || samples.is_null() || frame_count == 0 {
            return;
        }
        // SAFETY: the worker keeps this boxed context alive until the bridge
        // signals completion and releases its session.
        let context = unsafe { &*(context.cast::<CallbackContext>()) };
        // SAFETY: the bridge guarantees `frame_count` contiguous float samples
        // for the duration of this callback.
        let audio = unsafe { std::slice::from_raw_parts(samples, frame_count as usize) }.to_vec();
        let Some(channels) = NonZero::new(1_u16) else {
            return;
        };
        let rounded_rate = sample_rate.round();
        if !(1.0..=u32::MAX as f64).contains(&rounded_rate) {
            return;
        }
        let Some(rate) = NonZero::new(rounded_rate as u32) else {
            return;
        };
        context
            .player
            .append(rodio::buffer::SamplesBuffer::new(channels, rate, audio));
        context.tts_active.store(true, Ordering::Release);
    }

    unsafe extern "C" fn completion_callback(context: *mut c_void, error: *const c_char) {
        if context.is_null() {
            return;
        }
        // SAFETY: the worker owns the callback context until this notification.
        let context = unsafe { &*(context.cast::<CallbackContext>()) };
        let result = if error.is_null() {
            Ok(())
        } else {
            // SAFETY: the bridge supplies a NUL-terminated string that remains
            // valid for the duration of this callback.
            let message = unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned();
            Err(message)
        };
        *lock(&context.completion) = Some(result);
        context.completion_ready.notify_all();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_worker(
        voice: String,
        language: String,
        speech_rate: f32,
        text_rx: Receiver<String>,
        tts_active: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        output_device: Option<String>,
    ) {
        let sink_handle = match open_output_sink_by_name(output_device.as_deref()) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("buzz-desktop: Siri TTS audio output failed: {error}");
                super::super::drain_until_shutdown(text_rx, &shutdown);
                return;
            }
        };
        let player = Arc::new(rodio::Player::connect_new(sink_handle.mixer()));

        loop {
            if shutdown.load(Ordering::Acquire) {
                player.clear();
                break;
            }
            clear_cancelled(&text_rx, &tts_active, &cancel, &player);

            let raw_text = match text_rx.recv_timeout(RECV_TIMEOUT) {
                Ok(text) => text,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if player.empty() {
                        tts_active.store(false, Ordering::Release);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let text = preprocess_for_tts(&raw_text);
            if text.is_empty() {
                continue;
            }

            let mut context = Box::new(CallbackContext {
                player: Arc::clone(&player),
                tts_active: Arc::clone(&tts_active),
                completion: Mutex::new(None),
                completion_ready: Condvar::new(),
            });
            let context_pointer = (&mut *context as *mut CallbackContext).cast();
            let session = match Session::new(context_pointer, audio_callback, completion_callback) {
                Ok(session) => session,
                Err(error) => {
                    eprintln!("buzz-desktop: Siri TTS session failed: {error}");
                    continue;
                }
            };
            if let Err(error) = session.synthesize(&text, &language, &voice, speech_rate) {
                eprintln!("buzz-desktop: Siri TTS request failed: {error}");
                continue;
            }

            let mut completion = lock(&context.completion);
            while completion.is_none() {
                let waited = context
                    .completion_ready
                    .wait_timeout(completion, MONITOR_TICK)
                    .unwrap_or_else(PoisonError::into_inner);
                completion = waited.0;
                if shutdown.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                    session.cancel();
                    break;
                }
            }
            if let Some(Err(error)) = completion.take() {
                eprintln!("buzz-desktop: Siri TTS synthesis failed: {error}");
            }
            drop(completion);
            drop(session);

            if shutdown.load(Ordering::Acquire) {
                player.clear();
                break;
            }
            clear_cancelled(&text_rx, &tts_active, &cancel, &player);
        }
        tts_active.store(false, Ordering::Release);
    }

    fn clear_cancelled(
        text_rx: &Receiver<String>,
        tts_active: &AtomicBool,
        cancel: &AtomicBool,
        player: &rodio::Player,
    ) {
        if cancel.swap(false, Ordering::AcqRel) {
            player.clear();
            player.play();
            while text_rx.try_recv().is_ok() {}
            tts_active.store(false, Ordering::Release);
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(target_os = "macos")]
pub(super) use playback::run_worker;

#[cfg(not(target_os = "macos"))]
pub fn list_voices(_language_prefix: &str) -> Result<Vec<SiriVoice>, String> {
    Ok(Vec::new())
}

#[cfg(not(target_os = "macos"))]
pub fn is_voice_installed(_name: &str, _language: &str) -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(target_os = "macos"))]
pub fn trigger_download(_name: &str, _language: &str) -> Result<(), String> {
    Err("Siri TTS is only available on macOS".into())
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
pub(super) fn run_worker(
    _voice: String,
    _language: String,
    _speech_rate: f32,
    text_rx: std::sync::mpsc::Receiver<String>,
    _tts_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _output_device: Option<String>,
) {
    eprintln!("buzz-desktop: Siri TTS is only available on macOS");
    super::drain_until_shutdown(text_rx, &shutdown);
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::{
        ffi::{c_char, c_void, CStr},
        sync::{Condvar, Mutex},
        time::Duration,
    };

    struct Capture {
        samples: Mutex<Vec<f32>>,
        completion: Mutex<Option<Result<(), String>>>,
        ready: Condvar,
    }

    unsafe extern "C" fn capture_audio(
        context: *mut c_void,
        samples: *const f32,
        frames: u32,
        _sample_rate: f64,
    ) {
        // SAFETY: the test keeps Capture alive until synthesis completion.
        let capture = unsafe { &*(context.cast::<Capture>()) };
        // SAFETY: the bridge owns this contiguous buffer for the callback.
        let incoming = unsafe { std::slice::from_raw_parts(samples, frames as usize) };
        capture.samples.lock().unwrap().extend_from_slice(incoming);
    }

    unsafe extern "C" fn capture_completion(context: *mut c_void, error: *const c_char) {
        // SAFETY: the test keeps Capture alive until synthesis completion.
        let capture = unsafe { &*(context.cast::<Capture>()) };
        let result = if error.is_null() {
            Ok(())
        } else {
            // SAFETY: the bridge error is NUL-terminated for this callback.
            Err(unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned())
        };
        *capture.completion.lock().unwrap() = Some(result);
        capture.ready.notify_all();
    }

    #[test]
    #[ignore = "queries the private macOS Siri voice services"]
    fn live_voice_catalog_contains_installed_voice() {
        let voices = super::list_voices("en").expect("Siri voice discovery should succeed");
        assert!(
            !voices.is_empty(),
            "expected at least one English Siri voice"
        );
        assert!(
            voices
                .iter()
                .any(|voice| voice.availability == super::SiriVoiceAvailability::Installed),
            "expected at least one downloaded English Siri voice"
        );
    }

    #[test]
    #[ignore = "streams speech from the private macOS Siri TTS service"]
    fn live_synthesis_streams_decoded_pcm() {
        let mut capture = Box::new(Capture {
            samples: Mutex::new(Vec::new()),
            completion: Mutex::new(None),
            ready: Condvar::new(),
        });
        let pointer = (&mut *capture as *mut Capture).cast();
        let session = super::Session::new(pointer, capture_audio, capture_completion).unwrap();
        let voice = std::env::var("BUZZ_SIRI_TEST_VOICE").unwrap_or_else(|_| "Aaron".into());
        session
            .synthesize(
                "Streaming speech should begin before the complete sentence is ready.",
                "en-US",
                &voice,
                1.0,
            )
            .unwrap();

        let completion = capture.completion.lock().unwrap();
        let (mut completion, timeout) = capture
            .ready
            .wait_timeout_while(completion, Duration::from_secs(15), |value| value.is_none())
            .unwrap();
        assert!(!timeout.timed_out(), "Siri synthesis timed out");
        completion.take().unwrap().unwrap();
        drop(completion);
        drop(session);
        assert!(
            capture.samples.lock().unwrap().len() > 4_800,
            "expected decoded Siri PCM"
        );
    }
}
