//! Whisper models (download, verify, load) and transcription.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use asides_core::markdown::{Segment, Word};
use asides_core::models::ModelSpec;
use sha2::{Digest, Sha256};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Clone, PartialEq)]
pub enum ModelState {
    Ready,
    Downloading { progress: f32 },
    Failed(String),
}

/// Tracks each model's availability, and downloads one when the user asks (never on its own).
pub struct ModelManager {
    states: Mutex<HashMap<&'static str, ModelState>>,
    changed: Condvar,
    notify: Box<dyn Fn(&'static str, ModelState) + Send + Sync>,
}

impl ModelManager {
    pub fn new(notify: impl Fn(&'static str, ModelState) + Send + Sync + 'static) -> Arc<ModelManager> {
        Arc::new(ModelManager { states: Mutex::new(HashMap::new()), changed: Condvar::new(), notify: Box::new(notify) })
    }

    /// The model's state: ready on disk, downloading, failed, or `None` (not there).
    pub fn state(&self, id: &str) -> Option<ModelState> {
        let spec = ModelSpec::find(id);
        match self.states.lock().unwrap().get(spec.id) {
            Some(state @ (ModelState::Downloading { .. } | ModelState::Failed(_))) => Some(state.clone()),
            _ => spec.path().is_file().then_some(ModelState::Ready),
        }
    }

    /// Downloads the model, unless it's on disk or being downloaded.
    pub fn download(self: &Arc<Self>, id: &str) {
        let spec = ModelSpec::find(id);
        let mut states = self.states.lock().unwrap();
        if matches!(states.get(spec.id), Some(ModelState::Downloading { .. })) {
            return;
        }
        if spec.path().is_file() {
            states.insert(spec.id, ModelState::Ready);
            drop(states);
            self.set(spec.id, ModelState::Ready);
            return;
        }
        states.insert(spec.id, ModelState::Downloading { progress: 0.0 });
        drop(states);
        self.set(spec.id, ModelState::Downloading { progress: 0.0 });
        let this = self.clone();
        std::thread::spawn(move || {
            let result =
                download(spec, &spec.path(), |progress| this.set(spec.id, ModelState::Downloading { progress }));
            this.set(
                spec.id,
                match result {
                    Ok(()) => ModelState::Ready,
                    Err(err) => ModelState::Failed(format!("Could not download the voice model: {err:#}")),
                },
            );
        });
    }

    /// The model's path once it's on disk: at once, or after the download under way. It never
    /// starts one.
    pub fn wait_ready(self: &Arc<Self>, id: &str) -> Result<PathBuf> {
        let spec = ModelSpec::find(id);
        let mut states = self.states.lock().unwrap();
        loop {
            if spec.path().is_file() {
                return Ok(spec.path());
            }
            match states.get(spec.id) {
                Some(ModelState::Downloading { .. }) => states = self.changed.wait(states).unwrap(),
                Some(ModelState::Failed(msg)) => bail!("{msg}"),
                _ => bail!(
                    "There's no voice model yet: click the microphone in asides, or download one in the asides window \
                     (Settings)"
                ),
            }
        }
    }

    fn set(&self, id: &'static str, state: ModelState) {
        self.states.lock().unwrap().insert(id, state.clone());
        self.changed.notify_all();
        (self.notify)(id, state);
    }
}

fn download(spec: &ModelSpec, path: &Path, progress: impl Fn(f32)) -> Result<()> {
    fs::create_dir_all(path.parent().context("model path has no parent")?)?;
    let partial = path.with_extension("part");
    let response = ureq::get(&spec.url()).call().context("request failed")?;
    let mut reader = response.into_body().into_with_config().limit(u64::MAX).reader();
    let mut file = File::create(&partial)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    let mut last_report = Instant::now();
    loop {
        let n = reader.read(&mut buf).context("download interrupted")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if last_report.elapsed() > Duration::from_millis(500) {
            progress(done as f32 / spec.size as f32);
            last_report = Instant::now();
        }
    }
    file.sync_all()?;
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != spec.sha256 {
        let _ = fs::remove_file(&partial);
        bail!("checksum mismatch for {}", spec.file);
    }
    fs::rename(&partial, path)?;
    Ok(())
}

/// Whisper's language detection, above this, is sure (and the language is remembered).
const SURE: f32 = 0.8;

pub struct Transcriber {
    pub model_id: String,
    context: WhisperContext,
    threads: i32,
    /// The language of the last recording whose language Whisper was sure of.
    recent: Option<String>,
}

impl Transcriber {
    pub fn load(model_id: &str, path: &Path) -> Result<Transcriber> {
        whisper_rs::install_logging_hooks();
        let path = path.to_str().context("model path is not UTF-8")?;
        let context = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .context("could not load the speech model")?;
        let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4) as i32;
        Ok(Transcriber { model_id: model_id.to_string(), context, threads, recent: None })
    }

    /// Verbatim transcription of 16 kHz mono audio, in the most likely of `languages` (empty: any).
    pub fn transcribe(&mut self, pcm: &[f32], languages: &[String]) -> Result<Vec<Segment>> {
        // Whisper needs at least a second of audio; pad short clips with silence.
        let mut padded;
        let pcm = if pcm.len() < 17_600 {
            padded = pcm.to_vec();
            padded.resize(17_600, 0.0);
            &padded[..]
        } else {
            pcm
        };
        let mut state = self.context.create_state()?;
        // English-only models know no other language: nothing to detect.
        let english = ["en".to_string()];
        let languages = if ModelSpec::find(&self.model_id).english { &english[..] } else { languages };
        let language = match languages {
            [only] => only.clone(),
            _ => {
                state.pcm_to_mel(pcm, self.threads as usize)?;
                let (_, probabilities) = state.lang_detect(0, self.threads as usize)?;
                let (language, sure) = choose_language(&probabilities, languages, self.recent.as_deref())?;
                if sure {
                    self.recent = Some(language.clone());
                }
                language
            }
        };
        if whisper_rs::get_lang_id(&language).is_none() {
            bail!("Unknown language \"{language}\" in config.toml (use Whisper codes such as \"en\" or \"ro\")");
        }

        let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
        params.set_language(Some(&language));
        params.set_n_threads(self.threads);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Each word's time: a pin goes after the word said as it was placed.
        params.set_token_timestamps(true);
        state.full(params, pcm)?;

        let eot = self.context.token_eot();
        let segments = state
            .as_iter()
            .map(|s| -> Result<Segment> {
                let (start, end) = (s.start_timestamp() as f32 / 100.0, s.end_timestamp() as f32 / 100.0);
                let text = s.to_str_lossy()?.into_owned();
                let tokens = (0..s.n_tokens()).filter_map(|i| s.get_token(i)).filter(|t| t.token_id() < eot);
                let tokens: Vec<(f32, Vec<u8>)> = tokens
                    .filter_map(|t| Some((t.token_data().t0 as f32 / 100.0, t.to_bytes().ok()?.to_vec())))
                    .collect();
                let words = words(&tokens, start, end).filter(|w| same_text(w, &text)).unwrap_or_default();
                Ok(Segment { start, end, text, words })
            })
            .collect::<Result<_>>()?;
        Ok(segments)
    }
}

/// A segment's tokens (their start and bytes: a letter can span two) as words, each starting at
/// its first token (a token starting with a space starts a word), in order and within the segment.
fn words(tokens: &[(f32, Vec<u8>)], start: f32, end: f32) -> Option<Vec<Word>> {
    let mut words: Vec<(f32, Vec<u8>)> = Vec::new();
    for (t0, bytes) in tokens {
        match words.last_mut() {
            Some((_, word)) if !bytes.starts_with(b" ") => word.extend_from_slice(bytes),
            _ => {
                let after = words.last().map_or(start, |w| w.0);
                words.push((t0.clamp(after, end.max(after)), bytes.clone()));
            }
        }
    }
    (!words.is_empty()).then(|| {
        words
            .into_iter()
            .map(|(start, bytes)| Word { start, text: String::from_utf8_lossy(&bytes).into_owned() })
            .collect()
    })
}

/// The words make the segment's text (else they're left out: it's said as one).
fn same_text(words: &[Word], text: &str) -> bool {
    let joined: String = words.iter().map(|w| w.text.as_str()).collect();
    joined.split_whitespace().eq(text.split_whitespace())
}

/// The most likely language among `allowed` (any language Whisper knows when empty), and whether
/// Whisper is sure of it. Unsure (a short clip, an accent), the language it was last sure of wins
/// when it's still a fair candidate: otherwise English said with an accent can come out written
/// in another language, Whisper writing in the language it detects.
fn choose_language(probabilities: &[f32], allowed: &[String], recent: Option<&str>) -> Result<(String, bool)> {
    let probability = |id: i32| probabilities.get(id as usize).copied().unwrap_or(0.0);
    let candidates: Vec<i32> = if allowed.is_empty() {
        (0..=whisper_rs::get_lang_max_id()).collect()
    } else {
        allowed
            .iter()
            .map(|code| {
                whisper_rs::get_lang_id(code).with_context(|| {
                    format!("Unknown language \"{code}\" in config.toml (use Whisper codes such as \"en\" or \"ro\")")
                })
            })
            .collect::<Result<_>>()?
    };
    let total: f32 = candidates.iter().map(|&id| probability(id)).sum();
    let best = candidates
        .iter()
        .copied()
        .max_by(|a, b| probability(*a).total_cmp(&probability(*b)))
        .context("no languages")?;
    // Among the candidates: with `languages` set, how sure it is between those.
    let sure = probability(best) >= SURE * total.max(f32::EPSILON);
    let fallback = recent
        .and_then(whisper_rs::get_lang_id)
        .filter(|&id| !sure && candidates.contains(&id) && probability(id) >= 0.1 * total);
    let chosen = fallback.unwrap_or(best);
    Ok((whisper_rs::get_lang_str(chosen).context("unknown language id")?.to_string(), sure))
}

/// True for recordings with no audible sound (e.g. the microphone delivered silence).
pub fn is_silent(pcm: &[f32]) -> bool {
    pcm.chunks(480).all(|frame| {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        rms < 0.002
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use asides_core::markdown::plain_text;

    fn fixture(name: &str) -> Vec<f32> {
        read_wav(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name))
    }

    fn read_wav(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
    }

    /// The words of `reference` that `heard` got wrong (substituted, left out or added), and how
    /// many words `reference` has. Case, punctuation, hyphens, numbers in words or digits ("five",
    /// "5") and a compound written apart ("in line" for "inline") aren't mistakes.
    fn word_errors(reference: &str, heard: &str) -> (usize, usize) {
        const NUMBERS: [&str; 13] =
            ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
        let words = |text: &str| -> Vec<String> {
            text.to_lowercase()
                .replace('-', " ")
                .split_whitespace()
                .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>())
                .filter(|w| !w.is_empty())
                .map(|w| NUMBERS.iter().position(|n| *n == w).map_or(w, |i| i.to_string()))
                .collect()
        };
        let a = words(reference);
        let mut b = Vec::new();
        let mut heard = words(heard).into_iter().peekable();
        while let Some(word) = heard.next() {
            match heard.peek().map(|next| format!("{word}{next}")).filter(|joined| a.contains(joined)) {
                Some(joined) => {
                    heard.next();
                    b.push(joined);
                }
                None => b.push(word),
            }
        }
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, x) in a.iter().enumerate() {
            let mut diagonal = row[0];
            row[0] = i + 1;
            for (j, y) in b.iter().enumerate() {
                let next = (diagonal + usize::from(x != y)).min(row[j] + 1).min(row[j + 1] + 1);
                diagonal = row[j + 1];
                row[j + 1] = next;
            }
        }
        (row[b.len()], a.len())
    }

    #[test]
    fn counts_word_errors() {
        assert_eq!(word_errors("The Save button, doesn't work.", "the save button doesn't work"), (0, 5));
        assert_eq!(word_errors("a b c d", "a x c"), (2, 4));
        assert_eq!(word_errors("dark-mode colors", "dark mode colours"), (1, 3));
        assert_eq!(word_errors("show them inline for five seconds", "show them in line for 5 seconds"), (0, 6));
    }

    /// Compares speech models on recordings with what was said: `ASIDES_BENCH` is a folder of
    /// `<name>.wav` (16 kHz mono) with `<name>.txt` beside each, `ASIDES_BENCH_MODELS` a list of
    /// `id=path` (`id:en=path` makes a multilingual model write English). Prints each model's word
    /// error rate and time.
    #[test]
    #[ignore = "needs recordings and models (ASIDES_BENCH, ASIDES_BENCH_MODELS)"]
    fn compares_models() {
        let dir = PathBuf::from(std::env::var("ASIDES_BENCH").expect("ASIDES_BENCH"));
        let mut clips: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .collect();
        clips.sort();
        let mut table = Vec::new();
        for model in std::env::var("ASIDES_BENCH_MODELS").expect("ASIDES_BENCH_MODELS").split(',') {
            let (name, path) = model.split_once('=').expect("id=path");
            let (id, languages) = match name.split_once(':') {
                Some((id, language)) => (id, vec![language.to_string()]),
                None => (name, Vec::new()),
            };
            let mut transcriber = Transcriber::load(id, Path::new(path)).unwrap();
            // Loaded onto the GPU before timing.
            transcriber.transcribe(&read_wav(&clips[0]), &languages).unwrap();
            let (mut errors, mut words, mut time) = (0, 0, Duration::ZERO);
            for clip in &clips {
                let reference = fs::read_to_string(clip.with_extension("txt")).unwrap();
                let pcm = read_wav(clip);
                let started = Instant::now();
                let heard = plain_text(&transcriber.transcribe(&pcm, &languages).unwrap());
                time += started.elapsed();
                let (e, n) = word_errors(&reference, &heard);
                errors += e;
                words += n;
                println!("{name} {}: {e}/{n} {heard}", clip.file_stem().unwrap().to_string_lossy());
            }
            table.push(format!(
                "{name}: {:.1}% of {words} words wrong, {:.2?} a clip",
                100.0 * errors as f64 / words as f64,
                time / clips.len() as u32
            ));
        }
        println!("\n{}", table.join("\n"));
    }

    #[test]
    fn chooses_the_most_likely_allowed_language() {
        let id = |code: &str| whisper_rs::get_lang_id(code).unwrap() as usize;
        let mut probabilities = vec![0.0f32; whisper_rs::get_lang_max_id() as usize + 1];
        probabilities[id("it")] = 0.6;
        probabilities[id("ro")] = 0.3;
        probabilities[id("en")] = 0.1;
        let allowed = |codes: &[&str]| codes.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let choose = |p: &[f32], allowed: &[String], recent| choose_language(p, allowed, recent).unwrap();
        assert_eq!(choose(&probabilities, &allowed(&["en", "ro"]), None), ("ro".to_string(), false));
        assert_eq!(choose(&probabilities, &[], None), ("it".to_string(), false));
        assert!(choose_language(&probabilities, &allowed(&["en", "klingon"]), None).is_err());
        // Unsure, the language it was last sure of, while it's a fair candidate.
        assert_eq!(choose(&probabilities, &allowed(&["en", "ro"]), Some("en")), ("en".to_string(), false));
        assert_eq!(choose(&probabilities, &[], Some("de")), ("it".to_string(), false));
        probabilities[id("en")] = 0.95;
        probabilities[id("it")] = 0.03;
        probabilities[id("ro")] = 0.02;
        assert_eq!(choose(&probabilities, &[], Some("ro")), ("en".to_string(), true));
    }

    #[test]
    fn detects_silence() {
        assert!(is_silent(&vec![0.0; 16_000]));
        assert!(is_silent(&vec![0.001; 16_000]), "a faint hum is not sound");
        let speech_level: Vec<f32> = (0..16_000).map(|i| 0.1 * (i as f32 * 0.05).sin()).collect();
        assert!(!is_silent(&speech_level));
    }

    #[test]
    #[ignore = "needs the large-v3-turbo model in ~/Library/Application Support/asides/models"]
    fn transcribes_romanian_and_english_verbatim() {
        let spec = ModelSpec::find("large-v3-turbo");
        let mut transcriber = Transcriber::load(spec.id, &spec.path()).unwrap();

        let started = Instant::now();
        let languages = ["ro".to_string(), "en".to_string()];
        let ro = transcriber.transcribe(&fixture("ro_bug.wav"), &languages).unwrap();
        println!("ro ({:.2?}): {}", started.elapsed(), plain_text(&ro));
        assert!(plain_text(&ro).contains("Butonul de salvare nu face nimic"));
        assert!(plain_text(&ro).contains("ecrane mici"));

        let en = transcriber.transcribe(&fixture("en_bug.wav"), &[]).unwrap();
        println!("en: {}", plain_text(&en));
        assert!(plain_text(&en).contains("padding on the summary card"));
        assert!(en.windows(2).all(|w| w[0].start <= w[1].start), "segments are in order");
        assert!(en.last().unwrap().end > 9.0, "timestamps cover the clip");
        // Each word timed, in order and within its segment, diacritics whole.
        for s in ro.iter().chain(&en) {
            println!("{:?}", s.words.iter().map(|w| format!("{:.2} {}", w.start, w.text.trim())).collect::<Vec<_>>());
            assert!(!s.words.is_empty() && same_text(&s.words, &s.text), "{s:?}");
            assert!(s.words.windows(2).all(|w| w[0].start <= w[1].start), "{s:?}");
            assert!(s.words.iter().all(|w| (s.start..=s.end).contains(&w.start)), "{s:?}");
        }
    }

    #[test]
    #[ignore = "downloads 190 MB"]
    fn downloads_and_verifies_a_model() {
        let spec = ModelSpec::find("small");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(spec.file);
        let reports = std::sync::atomic::AtomicUsize::new(0);
        download(spec, &path, |_| {
            reports.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
        .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), spec.size);
        assert!(reports.load(std::sync::atomic::Ordering::Relaxed) > 0);
    }
}
