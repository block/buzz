//! Standalone Pocket TTS quality harness.
//!
//! This deliberately imports the production Pocket TTS and preprocessing
//! modules by path. It exercises the same model configuration, prompt
//! preparation, text cleanup, sentence splitting, chunking, output clamping,
//! and boundary treatment as huddle TTS without joining a huddle.

use std::env;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../../../desktop/src-tauri/src/huddle/pocket.rs"]
mod pocket;

#[path = "../../../desktop/src-tauri/src/huddle/preprocessing.rs"]
mod preprocessing;

use pocket::{
    load_text_to_speech, load_voice_style, PocketTts, PromptPrefix, VoiceStyle, DEFAULT_VOICE,
    SAMPLE_RATE, VOICE_FILE_EXT,
};
use preprocessing::{preprocess_for_tts, split_sentences};

const DEFAULT_OUTPUT: &str = "/tmp/buzz-tts.wav";
const DEFAULT_CORPUS_DIR: &str = "/tmp/buzz-tts-corpus";
const SYNTH_STEPS: usize = 1;
const MAX_CHUNK_CHARS: usize = 200;
const INTER_CHUNK_SILENCE: Duration = Duration::from_millis(100);
const LEAD_IN: Duration = Duration::from_millis(20);
const FADE_OUT: Duration = Duration::from_millis(8);
const PREFIX_SEARCH_START: Duration = Duration::from_millis(150);
const PREFIX_SEARCH_END: Duration = Duration::from_millis(1_200);
const PREFIX_SILENCE: Duration = Duration::from_millis(80);
const PREFIX_RETAINED_SILENCE: Duration = Duration::from_millis(50);
const PREFIX_SILENCE_PEAK_RATIO: f32 = 0.02;

const DEFAULT_TEXT: &str = concat!(
    "This sentence has five words. Here are five more words. Five-word sentences are fine. ",
    "But several together become monotonous. Listen to what is happening. ",
    "The writing is getting boring. The sound of it drones. It’s like a stuck record. ",
    "The ear demands some variety. Now listen. ",
    "I vary the sentence length, and I create music. Music. The writing sings. ",
    "It has a pleasant rhythm, a lilt, a harmony. I use short sentences. ",
    "And I use sentences of medium length. ",
    "And sometimes, when I am certain the reader is rested, I will engage him with a sentence ",
    "of considerable length, a sentence that burns with energy and builds with all the impetus ",
    "of a crescendo, the roll of the drums, the crash of the cymbals–sounds that say listen to ",
    "this, it is important."
);

const SAY_REFERENCE_TEXT: &str = concat!(
    "On a clear autumn morning, the old harbor stirred beneath a pale blue sky. ",
    "Waves moved quietly against the wooden pier while distant bells marked the passing hour. ",
    "A curious traveler paused beside the water, took a slow breath, and began telling a story ",
    "with warmth, patience, and a little unexpected humor."
);

const CORPUS: &[&str] = &[
    "Yep.",
    "Sounds good to me.",
    "I'm doing well, thanks for asking.",
    "I've been working on the relay code all morning.",
    "I looked at the relay code this morning. The lease logic is solid. There's one race in the worker claim path, though. I'll write it up and send you a patch.",
    "Great question. The answer is it depends on the community size. For small ones, keep it simple.",
    "That's 42 open PRs right now — mostly small. I'll triage them after lunch.",
    "The function foo_bar calls baz_qux which returns a Result type.",
    "Check out https://example.com for more info.",
    "Dr. Smith went to the store. He bought 42 apples.",
];

#[derive(Debug)]
enum SayVoice {
    SystemDefault,
    Named(String),
}

#[derive(Debug)]
struct Args {
    text: Option<String>,
    output: Option<PathBuf>,
    model_dir: PathBuf,
    voice: Option<PathBuf>,
    say_voice: Option<SayVoice>,
    corpus: bool,
    play: bool,
    play_as_rendered: bool,
    speed: f64,
    prefix: PromptPrefix,
    analyze_prefix: bool,
    trim_prefix: bool,
    validate_prefix_stt: bool,
    split_long_clauses: bool,
    sentence_by_sentence: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;
    let model_dir = args.model_dir;
    let voice_path = match args.say_voice.as_ref() {
        Some(voice) => generate_say_reference(voice)?,
        None => args
            .voice
            .unwrap_or_else(|| model_dir.join(format!("{DEFAULT_VOICE}.{VOICE_FILE_EXT}"))),
    };

    eprintln!("Loading Pocket TTS from {} …", model_dir.display());
    let engine = load_text_to_speech(
        model_dir
            .to_str()
            .ok_or_else(|| format!("model path is not valid UTF-8: {}", model_dir.display()))?,
    )?;
    let voice = load_voice_style(&voice_path)?;
    let prefix_validator = if args.validate_prefix_stt {
        Some(load_prefix_validator()?)
    } else {
        None
    };

    if args.corpus {
        let output_dir = args
            .output
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CORPUS_DIR));
        render_corpus(
            &engine,
            &voice,
            &output_dir,
            &args.prefix,
            args.analyze_prefix,
            args.trim_prefix,
            prefix_validator.as_ref(),
            args.split_long_clauses,
            args.sentence_by_sentence,
            false,
            args.speed,
        )
    } else {
        let text = read_text(args.text)?;
        let output = args.output.unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT));
        render_one(
            &engine,
            &voice,
            &text,
            &output,
            &args.prefix,
            args.analyze_prefix,
            args.trim_prefix,
            prefix_validator.as_ref(),
            args.split_long_clauses,
            args.sentence_by_sentence,
            args.play && args.play_as_rendered,
            args.speed,
        )?;
        eprintln!("\nWrote {}", output.display());
        if args.play && !args.play_as_rendered {
            play_audio(&output, args.speed)?;
        }
        Ok(())
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut text_parts = Vec::new();
    let mut output = None;
    let mut corpus = false;
    let mut play = true;
    let mut play_as_rendered = true;
    let mut speed = 1.0;
    let mut prefix = PromptPrefix::Period;
    let mut analyze_prefix = false;
    let mut trim_prefix = false;
    let mut validate_prefix_stt = false;
    let mut split_long_clauses = false;
    let mut sentence_by_sentence = false;
    let mut model_dir = dirs::home_dir()
        .map(|path| path.join(".buzz/models/pocket-tts"))
        .ok_or_else(|| "could not determine the home directory; pass --model-dir".to_string())?;
    let mut voice = None;
    let mut say_voice = None;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--corpus" => corpus = true,
            "--no-play" => play = false,
            "--play-as-rendered" => play_as_rendered = true,
            "--play-after-render" => play_as_rendered = false,
            "--speed" => {
                let value = required_value(&mut args, &arg)?;
                speed = value
                    .parse::<f64>()
                    .map_err(|_| format!("invalid speed factor: {value}"))?;
                if !(0.5..=2.0).contains(&speed) {
                    return Err("--speed must be between 0.5 and 2.0".to_string());
                }
            }
            "--prefix" => {
                let value = required_value(&mut args, &arg)?;
                prefix = parse_prefix(&value)?;
            }
            "--prefix-spaces" => {
                let value = required_value(&mut args, &arg)?;
                let count = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid prefix space count: {value}"))?;
                if count > 64 {
                    return Err("--prefix-spaces must be between 0 and 64".to_string());
                }
                prefix = if count == 0 {
                    PromptPrefix::None
                } else {
                    PromptPrefix::Spaces(count)
                };
            }
            "--prefix-text" => {
                let value = required_value(&mut args, &arg)?;
                if value.len() > 64 {
                    return Err("--prefix-text must be at most 64 bytes".to_string());
                }
                prefix = if value.is_empty() {
                    PromptPrefix::None
                } else {
                    PromptPrefix::Custom(value)
                };
            }
            "--analyze-prefix" => analyze_prefix = true,
            "--trim-prefix" => trim_prefix = true,
            "--validate-prefix-stt" => validate_prefix_stt = true,
            "--split-long-clauses" => split_long_clauses = true,
            "--sentence-by-sentence" => sentence_by_sentence = true,
            "-o" | "--output" => {
                output = Some(PathBuf::from(required_value(&mut args, &arg)?));
            }
            "--model-dir" => {
                model_dir = PathBuf::from(required_value(&mut args, &arg)?);
            }
            "--voice" => {
                voice = Some(PathBuf::from(required_value(&mut args, &arg)?));
            }
            "--say-voice" => {
                say_voice = Some(SayVoice::SystemDefault);
            }
            "--say-voice-name" => {
                say_voice = Some(SayVoice::Named(required_value(&mut args, &arg)?));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => text_parts.push(arg),
        }
    }

    if corpus && !text_parts.is_empty() {
        return Err("--corpus cannot be combined with input text".to_string());
    }
    if corpus && speed != 1.0 {
        return Err("--speed only applies to automatic playback, not --corpus".to_string());
    }
    if voice.is_some() && say_voice.is_some() {
        return Err("--voice and --say-voice cannot be combined".to_string());
    }

    Ok(Args {
        text: (!text_parts.is_empty()).then(|| text_parts.join(" ")),
        output,
        model_dir,
        voice,
        say_voice,
        corpus,
        play,
        play_as_rendered,
        speed,
        prefix,
        analyze_prefix,
        trim_prefix,
        validate_prefix_stt,
        split_long_clauses,
        sentence_by_sentence,
    })
}

fn required_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_prefix(value: &str) -> Result<PromptPrefix, String> {
    match value {
        "spaces" => Ok(PromptPrefix::Spaces(8)),
        "period" => Ok(PromptPrefix::Period),
        "none" => Ok(PromptPrefix::None),
        _ => Err(format!(
            "invalid prefix strategy: {value}; expected spaces, period, or none"
        )),
    }
}

fn print_help() {
    println!(
        "\
Exercise Buzz's production Pocket TTS path without a huddle.

Usage:
  scripts/test-tts.sh [OPTIONS] [TEXT...]
  echo \"Text from stdin\" | scripts/test-tts.sh [OPTIONS]

Options:
  --corpus             Render the built-in quality corpus
  -o, --output PATH    WAV path, or output directory with --corpus
  --model-dir PATH     Pocket TTS model directory
  --voice PATH         Reference voice WAV
  --say-voice          Clone the default macOS system voice
  --say-voice-name N   Clone a named built-in macOS voice
  --prefix STRATEGY    Chunk prefix: period, spaces, or none
  --prefix-spaces N    Use exactly N leading spaces, from 0 to 64
  --prefix-text TEXT   Use an exact literal prefix, up to 64 bytes
  --analyze-prefix     Report the prefix pause without removing it
  --trim-prefix        Remove a spoken prefix at its following silence
  --validate-prefix-stt
                       Check the proposed trim with offline speech recognition
  --split-long-clauses Split overlong sentences at commas
  --sentence-by-sentence
                       Render each sentence as its own TTS call
  --play-as-rendered   Start playback while later chunks render (default)
  --play-after-render  Wait for the complete WAV before playing
  --speed FACTOR       Pitch-preserving afplay rate, from 0.5 to 2.0
  --no-play            Write the WAV without playing it
  -h, --help           Show this help

Defaults:
  model:  ~/.buzz/models/pocket-tts
  output: {DEFAULT_OUTPUT}
  corpus: {DEFAULT_CORPUS_DIR}
  text:   built-in sentence-length passage
  prefix: period
  play:   as each chunk is rendered"
    );
}

fn read_text(argument: Option<String>) -> Result<String, String> {
    if let Some(text) = argument {
        return nonempty(text);
    }
    if io::stdin().is_terminal() {
        return Ok(DEFAULT_TEXT.to_string());
    }

    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| format!("read stdin: {error}"))?;
    if text.trim().is_empty() {
        Ok(DEFAULT_TEXT.to_string())
    } else {
        nonempty(text)
    }
}

fn generate_say_reference(voice: &SayVoice) -> Result<PathBuf, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = voice;
        return Err("--say-voice is supported only on macOS".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let voice_name = match voice {
            SayVoice::SystemDefault => None,
            SayVoice::Named(name) => Some(name.as_str()),
        };
        let safe_voice: String = voice_name
            .unwrap_or("system-default")
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let output = PathBuf::from(format!("/tmp/buzz-tts-say-{safe_voice}.wav"));
        eprintln!(
            "Generating macOS {} reference voice at {} …",
            voice_name.unwrap_or("system default"),
            output.display()
        );
        let mut command = Command::new("say");
        if let Some(name) = voice_name {
            command.args(["-v", name]);
        }
        let status = command
            .args(["-r", "175", "-o"])
            .arg(&output)
            .args(["--file-format=WAVE", "--data-format=LEI16@32000"])
            .arg(SAY_REFERENCE_TEXT)
            .status()
            .map_err(|error| format!("start macOS say command: {error}"))?;
        if status.success() {
            Ok(output)
        } else {
            Err(format!(
                "macOS say exited with {status}; check the voice name with: say -v '?'"
            ))
        }
    }
}

fn nonempty(text: String) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        Err("input text is empty".to_string())
    } else {
        Ok(text)
    }
}

fn play_audio(path: &Path, speed: f64) -> Result<(), String> {
    let status = audio_player(path, speed)?
        .status()
        .map_err(|error| format!("start audio player for {}: {error}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "audio player exited with {status} for {}",
            path.display()
        ))
    }
}

fn audio_player(path: &Path, speed: f64) -> Result<Command, String> {
    #[cfg(target_os = "macos")]
    let player = {
        let mut command = Command::new("afplay");
        command
            .args(["-r", &format!("{speed:.6}"), "-q", "1"])
            .arg(path);
        command
    };
    #[cfg(target_os = "linux")]
    let player = {
        if speed != 1.0 {
            return Err("--speed is currently supported only on macOS".to_string());
        }
        let mut command = Command::new("aplay");
        command.arg(path);
        command
    };
    #[cfg(target_os = "windows")]
    let player = {
        if speed != 1.0 {
            return Err("--speed is currently supported only on macOS".to_string());
        }
        let mut command = Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-Command",
                "(New-Object Media.SoundPlayer $args[0]).PlaySync()",
            ])
            .arg(path);
        command
    };

    Ok(player)
}

fn render_corpus(
    engine: &PocketTts,
    voice: &VoiceStyle,
    output_dir: &Path,
    prefix: &PromptPrefix,
    analyze_prefix: bool,
    trim_prefix: bool,
    prefix_validator: Option<&sherpa_onnx::OfflineRecognizer>,
    split_long_clauses: bool,
    sentence_by_sentence: bool,
    play_as_rendered: bool,
    speed: f64,
) -> Result<(), String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|error| format!("create {}: {error}", output_dir.display()))?;
    eprintln!(
        "Rendering {} clips into {} …\n",
        CORPUS.len(),
        output_dir.display()
    );

    for (index, text) in CORPUS.iter().enumerate() {
        let output = output_dir.join(format!("{:02}-{}.wav", index + 1, slug(text)));
        render_one(
            engine,
            voice,
            text,
            &output,
            prefix,
            analyze_prefix,
            trim_prefix,
            prefix_validator,
            split_long_clauses,
            sentence_by_sentence,
            play_as_rendered,
            speed,
        )?;
    }

    eprintln!("\nWrote {} clips to {}", CORPUS.len(), output_dir.display());
    Ok(())
}

fn render_one(
    engine: &PocketTts,
    voice: &VoiceStyle,
    raw_text: &str,
    output: &Path,
    prefix: &PromptPrefix,
    analyze_prefix: bool,
    trim_prefix: bool,
    prefix_validator: Option<&sherpa_onnx::OfflineRecognizer>,
    split_long_clauses: bool,
    sentence_by_sentence: bool,
    play_as_rendered: bool,
    speed: f64,
) -> Result<(), String> {
    let cleaned = preprocess_for_tts(raw_text);
    if cleaned.is_empty() {
        return Err(format!("preprocessing removed all input: {raw_text:?}"));
    }

    let sentences: Vec<String> = split_sentences(&cleaned)
        .into_iter()
        .filter(|sentence| !sentence.trim().is_empty())
        .collect();
    let sentences = if split_long_clauses {
        sentences
            .iter()
            .flat_map(|sentence| split_long_sentence_at_clauses(sentence, MAX_CHUNK_CHARS))
            .collect()
    } else {
        sentences
    };
    let chunks = if sentence_by_sentence {
        sentences
    } else {
        group_sentences_into_chunks(&sentences, MAX_CHUNK_CHARS)
    };

    eprintln!("Input:   {raw_text}");
    if cleaned != raw_text {
        eprintln!("Cleaned: {cleaned}");
    }
    eprintln!("Chunks:  {}", chunks.len());

    let started = Instant::now();
    let mut output_samples = Vec::new();
    let (playback_sender, playback_worker) = if play_as_rendered {
        let (sender, receiver) = mpsc::channel::<Vec<f32>>();
        let worker = thread::Builder::new()
            .name("tts-playback".to_string())
            .spawn(move || -> Result<(), String> {
                for (index, samples) in receiver.into_iter().enumerate() {
                    let playback_file = env::temp_dir().join(format!(
                        "buzz-tts-playback-{}-{index}.wav",
                        std::process::id()
                    ));
                    write_wav(&playback_file, &samples)?;
                    let result = play_audio(&playback_file, speed);
                    let _ = std::fs::remove_file(&playback_file);
                    result?;
                }
                Ok(())
            })
            .map_err(|error| format!("start playback worker: {error}"))?;
        (Some(sender), Some(worker))
    } else {
        (None, None)
    };

    for (index, chunk) in chunks.iter().enumerate() {
        eprintln!("  {}/{}: {chunk}", index + 1, chunks.len());
        let mut samples =
            engine.synth_chunk_with_prefix(chunk.trim(), "en", voice, SYNTH_STEPS, prefix)?;
        if analyze_prefix || trim_prefix || prefix_validator.is_some() {
            let boundary = find_spoken_prefix_boundary(&samples);
            match boundary {
                Some(boundary) => {
                    eprintln!(
                        "       prefix pause: {:.0}–{:.0}ms; trim at {:.0}ms",
                        samples_to_duration(boundary.silence_start).as_millis(),
                        samples_to_duration(boundary.silence_end).as_millis(),
                        samples_to_duration(boundary.trim_at()).as_millis(),
                    );
                }
                None => eprintln!("       warning: no prefix boundary found; audio left intact"),
            }

            if let Some(validator) = prefix_validator {
                let raw_transcript = transcribe(validator, &samples);
                let candidate = boundary
                    .map(|boundary| &samples[boundary.trim_at()..])
                    .unwrap_or(&samples);
                let transcript = transcribe(validator, candidate);
                let expected = first_word(chunk);
                let actual = first_word(&transcript);
                let raw_words = normalized_words(&raw_transcript);
                let raw_target = raw_words
                    .iter()
                    .position(|word| word == "buzz")
                    .and_then(|index| raw_words.get(index + 1));
                let matches_source =
                    actual == expected || raw_target.is_some_and(|raw_word| raw_word == &actual);
                let status = if matches_source { "PASS" } else { "FAIL" };
                eprintln!(
                    "       STT {status}: raw {raw_transcript:?} → trimmed {transcript:?}; \
                     expected first word {expected:?}"
                );
            }

            if trim_prefix {
                if let Some(boundary) = boundary {
                    samples.drain(..boundary.trim_at());
                }
            }
        }
        let mut treated_samples = Vec::new();
        append_production_audio_treatment(&mut treated_samples, samples);
        output_samples.extend_from_slice(&treated_samples);

        if let Some(sender) = playback_sender.as_ref() {
            sender
                .send(treated_samples)
                .map_err(|_| "playback worker stopped unexpectedly".to_string())?;
        }
    }

    let generation_elapsed = started.elapsed();
    write_wav(output, &output_samples)?;

    let audio_seconds = output_samples.len() as f64 / f64::from(SAMPLE_RATE);
    let rtf = (audio_seconds > 0.0)
        .then(|| generation_elapsed.as_secs_f64() / audio_seconds)
        .unwrap_or_default();
    eprintln!(
        "  {:.2}s audio in {:.2}s (RTF {:.2}) → {}",
        audio_seconds,
        generation_elapsed.as_secs_f64(),
        rtf,
        output.display()
    );

    drop(playback_sender);
    if let Some(worker) = playback_worker {
        worker
            .join()
            .map_err(|_| "playback worker panicked".to_string())??;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrefixBoundary {
    silence_start: usize,
    silence_end: usize,
}

impl PrefixBoundary {
    fn trim_at(self) -> usize {
        self.silence_end
            .saturating_sub(duration_samples(PREFIX_RETAINED_SILENCE))
    }
}

fn find_spoken_prefix_boundary(samples: &[f32]) -> Option<PrefixBoundary> {
    let peak = samples
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    if peak == 0.0 {
        return None;
    }

    let threshold = peak * PREFIX_SILENCE_PEAK_RATIO;
    let search_start = duration_samples(PREFIX_SEARCH_START).min(samples.len());
    let search_end = duration_samples(PREFIX_SEARCH_END).min(samples.len());
    let required_silence = duration_samples(PREFIX_SILENCE);
    let mut quiet_start = None;

    for index in search_start..search_end {
        if samples[index].abs() <= threshold {
            quiet_start.get_or_insert(index);
        } else if let Some(start) = quiet_start.take() {
            if index - start >= required_silence {
                return Some(PrefixBoundary {
                    silence_start: start,
                    silence_end: index,
                });
            }
        }
    }

    if let Some(start) = quiet_start {
        if search_end - start >= required_silence {
            return Some(PrefixBoundary {
                silence_start: start,
                silence_end: search_end,
            });
        }
    }

    None
}

fn samples_to_duration(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / f64::from(SAMPLE_RATE))
}

fn load_prefix_validator() -> Result<sherpa_onnx::OfflineRecognizer, String> {
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

    let model_dir = dirs::home_dir()
        .ok_or_else(|| "could not determine home directory for STT model".to_string())?
        .join(".buzz/models/parakeet-tdt-ctc-110m-en");
    let model = model_dir.join("model.int8.onnx");
    let tokens = model_dir.join("tokens.txt");
    if !model.exists() || !tokens.exists() {
        return Err(format!(
            "offline STT model not found at {}",
            model_dir.display()
        ));
    }

    eprintln!("Loading prefix validator from {} …", model_dir.display());
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.nemo_ctc.model = Some(model.to_string_lossy().into_owned());
    config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    config.model_config.num_threads = 2;
    config.model_config.debug = false;
    OfflineRecognizer::create(&config)
        .ok_or_else(|| "could not create offline STT recognizer".to_string())
}

fn transcribe(recognizer: &sherpa_onnx::OfflineRecognizer, samples: &[f32]) -> String {
    let mut padded = Vec::with_capacity(
        duration_samples(Duration::from_millis(400)).saturating_add(samples.len()),
    );
    padded.extend(std::iter::repeat_n(
        0.0,
        duration_samples(Duration::from_millis(100)),
    ));
    padded.extend_from_slice(samples);
    padded.extend(std::iter::repeat_n(
        0.0,
        duration_samples(Duration::from_millis(300)),
    ));

    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, &padded);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|result| result.text.trim().to_string())
        .unwrap_or_default()
}

fn first_word(text: &str) -> String {
    normalized_words(text)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        })
        .filter(|word: &String| !word.is_empty())
        .collect()
}

fn write_wav(output: &Path, samples: &[f32]) -> Result<(), String> {
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let output_path = output
        .to_str()
        .ok_or_else(|| format!("output path is not valid UTF-8: {}", output.display()))?;
    if !sherpa_onnx::write(output_path, samples, SAMPLE_RATE as i32) {
        return Err(format!("could not write {}", output.display()));
    }
    Ok(())
}

fn split_long_sentence_at_clauses(sentence: &str, max_chars: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut remaining = sentence.trim();

    while remaining.len() > max_chars {
        let split_at = remaining
            .char_indices()
            .take_while(|(index, _)| *index < max_chars)
            .filter(|(_, character)| matches!(character, ',' | ';' | ':' | '—' | '–'))
            .map(|(index, character)| index + character.len_utf8())
            .last();
        let Some(split_at) = split_at else {
            break;
        };
        let (head, tail) = remaining.split_at(split_at);
        let head = head.trim();
        if !head.is_empty() {
            parts.push(head.to_string());
        }
        remaining = tail.trim_start();
    }

    if !remaining.is_empty() {
        parts.push(remaining.to_string());
    }
    parts
}

/// Mirror the production clamp, fade-out, and per-chunk silence treatment.
fn append_production_audio_treatment(output: &mut Vec<f32>, samples: Vec<f32>) {
    let mut audio: Vec<f32> = samples
        .into_iter()
        .map(|sample| sample.clamp(-1.0, 1.0))
        .collect();
    let fade_samples = duration_samples(FADE_OUT).min(audio.len() / 2);
    for index in 0..fade_samples {
        let position = audio.len() - 1 - index;
        audio[position] *= index as f32 / fade_samples as f32;
    }

    let lead_in_samples = duration_samples(LEAD_IN);
    let gap_samples = duration_samples(INTER_CHUNK_SILENCE);
    output.extend(std::iter::repeat_n(0.0, lead_in_samples));
    output.extend(audio);
    output.extend(std::iter::repeat_n(
        0.0,
        gap_samples.saturating_sub(lead_in_samples),
    ));
}

fn duration_samples(duration: Duration) -> usize {
    (duration.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize
}

/// Match production's latency-aware, greedy sentence grouping.
fn group_sentences_into_chunks(sentences: &[String], max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    for (index, sentence) in sentences.iter().enumerate() {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        if index == 0 || chunks.is_empty() {
            chunks.push(sentence.to_string());
            continue;
        }
        let can_merge = chunks.len() > 1
            && chunks
                .last()
                .is_some_and(|chunk| chunk.len() + 1 + sentence.len() <= max_chars);
        if can_merge {
            if let Some(chunk) = chunks.last_mut() {
                chunk.push(' ');
                chunk.push_str(sentence);
            }
        } else {
            chunks.push(sentence.to_string());
        }
    }
    chunks
}

fn slug(text: &str) -> String {
    let mut slug = String::with_capacity(40);
    let mut previous_dash = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if slug.len() >= 40 {
            break;
        }
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tester_tests {
    use super::*;

    #[test]
    fn overlong_sentence_splits_at_last_clause_before_budget() {
        let sentence = "And sometimes, when I am certain the reader is rested, I will engage him with a sentence of considerable length, a sentence that burns with energy and builds with all the impetus of a crescendo, the roll of the drums, the crash of the cymbals–sounds that say listen to this, it is important.";
        let parts = split_long_sentence_at_clauses(sentence, MAX_CHUNK_CHARS);

        assert_eq!(parts.len(), 2);
        assert!(parts[0].ends_with("crescendo,"));
        assert!(parts[1].starts_with("the roll of the drums"));
        assert!(parts[0].len() <= MAX_CHUNK_CHARS);
    }

    #[test]
    fn trims_prefix_at_sustained_silence_and_retains_lead_in() {
        let mut samples = vec![0.2; duration_samples(Duration::from_millis(400))];
        samples.extend(vec![0.0; duration_samples(Duration::from_millis(100))]);
        samples.extend(vec![0.3; duration_samples(Duration::from_millis(500))]);

        let boundary = find_spoken_prefix_boundary(&samples).expect("prefix boundary");
        samples.drain(..boundary.trim_at());

        assert_eq!(
            samples_to_duration(boundary.silence_start),
            Duration::from_millis(400)
        );
        assert_eq!(
            samples_to_duration(boundary.silence_end),
            Duration::from_millis(500)
        );
        assert_eq!(
            samples_to_duration(boundary.trim_at()),
            Duration::from_millis(450)
        );
        assert_eq!(
            samples
                .iter()
                .take_while(|sample| sample.abs() < f32::EPSILON)
                .count(),
            duration_samples(PREFIX_RETAINED_SILENCE)
        );
        assert_eq!(samples.len(), duration_samples(Duration::from_millis(550)));
    }

    #[test]
    fn leaves_audio_intact_without_sustained_silence() {
        let original = vec![0.2; duration_samples(Duration::from_secs(1))];

        assert_eq!(find_spoken_prefix_boundary(&original), None);
    }
}
