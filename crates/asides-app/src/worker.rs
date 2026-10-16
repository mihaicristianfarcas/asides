//! Background thread that transcribes recordings and writes notes, one job at a time and in order.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use anyhow::{Context, Result};
use asides_core::frontmatter::{self, Metadata};
use asides_core::markdown::{self, Attachment};
use asides_core::notes::{self, Note};
use asides_core::project;
use asides_core::resample::resample;
use chrono::{DateTime, Local};

use crate::environment::Environment;
use crate::stt::{self, ModelManager, Transcriber};

const WHISPER_RATE: u32 = 16_000;
/// Unload the model after this long without work, to give the memory back.
const IDLE_UNLOAD: Duration = Duration::from_secs(15 * 60);

pub struct CapturedImage {
    /// Seconds since the recording started (0 for images attached while idle).
    pub offset: f32,
    pub at: DateTime<Local>,
    pub bytes: Vec<u8>,
    pub extension: String,
    /// Markdown shown after the image (what was measured on it).
    pub details: Option<String>,
}

pub struct RecordingJob {
    pub dir: PathBuf,
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub started: DateTime<Local>,
    /// `Some` starts a new note (with an optional title); `None` appends to the latest note.
    pub new_note: Option<Option<String>>,
    pub images: Vec<CapturedImage>,
    /// Text added at a moment of the recording (an annotation), as (seconds, Markdown).
    pub texts: Vec<(f32, String)>,
    pub environment: Environment,
    pub keep_audio: bool,
    pub model: String,
    pub languages: Vec<String>,
}

pub struct ImageJob {
    pub dir: PathBuf,
    pub image: CapturedImage,
    pub new_note: Option<Option<String>>,
    pub environment: Environment,
    pub model: String,
}

/// Text another app transcribed (a Wispr Flow dictation), added as an entry of its own.
pub struct DictationJob {
    pub dir: PathBuf,
    pub text: String,
    pub at: DateTime<Local>,
    pub source: &'static str,
    pub new_note: Option<Option<String>>,
    pub environment: Environment,
}

/// A Measure & Annotate session outside a recording: an entry of screenshots and text, in the
/// order they were made (images by `offset`, texts by their seconds).
pub struct ScreenJob {
    pub dir: PathBuf,
    pub at: DateTime<Local>,
    pub images: Vec<CapturedImage>,
    pub texts: Vec<(f32, String)>,
    pub new_note: Option<Option<String>>,
    pub environment: Environment,
    pub model: String,
}

pub enum Job {
    Recording(Box<RecordingJob>),
    Image(Box<ImageJob>),
    Dictation(Box<DictationJob>),
    Screen(Box<ScreenJob>),
}

pub enum Outcome {
    Appended { note: String, number: u32 },
    ImageAdded { note: String },
    NothingHeard,
    Failed(String),
}

pub fn spawn(models: Arc<ModelManager>, done: impl Fn(Outcome) + Send + 'static) -> Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    std::thread::Builder::new()
        .name("asides-worker".into())
        .spawn(move || {
            let mut transcriber: Option<Transcriber> = None;
            loop {
                let job = match rx.recv_timeout(IDLE_UNLOAD) {
                    Ok(job) => job,
                    Err(RecvTimeoutError::Timeout) => {
                        transcriber = None;
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                let outcome = match job {
                    Job::Recording(job) => process_recording(*job, &models, &mut transcriber),
                    Job::Image(job) => process_image(*job),
                    Job::Dictation(job) => process_dictation(*job),
                    Job::Screen(job) => process_screen(*job),
                };
                done(outcome.unwrap_or_else(|err| Outcome::Failed(format!("{err:#}"))));
            }
        })
        .expect("spawn worker thread");
    tx
}

fn process_recording(
    job: RecordingJob,
    models: &Arc<ModelManager>,
    transcriber: &mut Option<Transcriber>,
) -> Result<Outcome> {
    let pcm = resample(&job.samples, job.sample_rate, WHISPER_RATE);
    let segments = if stt::is_silent(&pcm) {
        if job.images.is_empty() && job.texts.is_empty() && pcm.iter().all(|s| *s == 0.0) && !pcm.is_empty() {
            return Ok(Outcome::Failed(
                "The microphone delivered only silence. Check System Settings → Privacy & Security → Microphone."
                    .into(),
            ));
        }
        Vec::new()
    } else {
        if transcriber.as_ref().is_none_or(|t| t.model_id != job.model) {
            *transcriber = None;
            let path = models.wait_ready(&job.model)?;
            *transcriber = Some(Transcriber::load(&job.model, &path)?);
        }
        transcriber.as_ref().unwrap().transcribe(&pcm, &job.languages)?
    };
    let text = markdown::plain_text(&segments);
    if text.is_empty() && job.images.is_empty() && job.texts.is_empty() {
        return Ok(Outcome::NothingHeard);
    }

    let note = target_note(&job.dir, &job.new_note, job.started, &job.environment, &whisper(&job.model))?;
    let number = note.next_entry_number();
    let mut attachments = Vec::new();
    for image in &job.images {
        attachments.push(save_image(&note, number, image)?);
    }
    attachments.extend(job.texts.iter().map(|(offset, text)| Attachment {
        offset: *offset,
        text: Some(text.clone()),
        ..Attachment::default()
    }));
    let audio = if job.keep_audio { save_audio(&note, number, job.started, &pcm).ok() } else { None };
    let entry = markdown::compose_entry(
        number,
        &job.started.format("%H:%M").to_string(),
        audio.as_deref(),
        &segments,
        &attachments,
    );
    note.append(&entry).context("could not write the note")?;
    Ok(Outcome::Appended { note: note.title, number })
}

fn process_image(job: ImageJob) -> Result<Outcome> {
    let note = target_note(&job.dir, &job.new_note, job.image.at, &job.environment, &whisper(&job.model))?;
    let number = note.next_entry_number().saturating_sub(1);
    let attachment = save_image(&note, number, &job.image)?;
    note.append(&attachment.markdown()).context("could not write the note")?;
    Ok(Outcome::ImageAdded { note: note.title })
}

fn process_dictation(job: DictationJob) -> Result<Outcome> {
    let transcription = format!("asides {} · {}", env!("CARGO_PKG_VERSION"), job.source);
    let note = target_note(&job.dir, &job.new_note, job.at, &job.environment, &transcription)?;
    let number = note.next_entry_number();
    let time = job.at.format("%H:%M").to_string();
    note.append(&markdown::compose_dictation_entry(number, &time, job.source, &job.text))
        .context("could not write the note")?;
    Ok(Outcome::Appended { note: note.title, number })
}

fn process_screen(job: ScreenJob) -> Result<Outcome> {
    let note = target_note(&job.dir, &job.new_note, job.at, &job.environment, &whisper(&job.model))?;
    let number = note.next_entry_number();
    let mut attachments = Vec::new();
    for image in &job.images {
        attachments.push(save_image(&note, number, image)?);
    }
    attachments.extend(job.texts.iter().map(|(offset, text)| Attachment {
        offset: *offset,
        text: Some(text.clone()),
        ..Attachment::default()
    }));
    // Stable: what was made at the same moment keeps its order.
    attachments.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let time = job.at.format("%H:%M").to_string();
    note.append(&markdown::compose_attachments_entry(number, &time, SCREEN_SOURCE, &attachments))
        .context("could not write the note")?;
    Ok(Outcome::Appended { note: note.title, number })
}

/// How entries made with Measure & Annotate (outside a recording) are marked.
pub const SCREEN_SOURCE: &str = "Screen";

/// The front matter's `transcription` for notes started by a recording.
fn whisper(model: &str) -> String {
    format!("asides {} · whisper {model} (local)", env!("CARGO_PKG_VERSION"))
}

/// The latest note, or a newly created one (with front matter) when asked for or when none exists.
/// A new note is titled as asked, or [`notes::DEFAULT_TITLE`]; never after what was said.
fn target_note(
    dir: &Path,
    new_note: &Option<Option<String>>,
    created: DateTime<Local>,
    environment: &Environment,
    transcription: &str,
) -> Result<Note> {
    if new_note.is_none()
        && let Some(note) = notes::latest_note(dir)
    {
        return Ok(note);
    }
    let title = new_note.clone().flatten().unwrap_or_else(|| notes::DEFAULT_TITLE.to_string());
    let metadata = Metadata {
        title: notes::sanitize_title(&title),
        created,
        directory: dir.to_path_buf(),
        git: project::git_info(dir),
        macos: environment.macos.clone(),
        browser: environment.browser.clone(),
        display: environment.display.clone(),
        transcription: transcription.to_string(),
    };
    notes::create_note(dir, created, &title, &frontmatter::render(&metadata)).context("could not create the note")
}

fn save_image(note: &Note, number: u32, image: &CapturedImage) -> Result<Attachment> {
    let dir = note.dir.join("images");
    fs::create_dir_all(&dir)?;
    let stem = format!("{number:02}-{}", image.at.format("%H%M%S"));
    let mut name = format!("{stem}.{}", image.extension);
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{stem}-{n}.{}", image.extension);
        n += 1;
    }
    fs::write(dir.join(&name), &image.bytes).context("could not save the image")?;
    Ok(Attachment {
        offset: image.offset,
        rel_path: format!("images/{name}"),
        label: image.at.format("%H:%M:%S").to_string(),
        text: image.details.clone(),
    })
}

/// Writes the recording as AAC (via the system's `afconvert`), falling back to WAV.
fn save_audio(note: &Note, number: u32, started: DateTime<Local>, pcm: &[f32]) -> Result<String> {
    let dir = note.dir.join("audio");
    fs::create_dir_all(&dir)?;
    let stem = format!("{number:02}-{}", started.format("%H%M%S"));
    let wav = dir.join(format!("{stem}.wav"));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec)?;
    for s in pcm {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    let m4a = dir.join(format!("{stem}.m4a"));
    let converted = Command::new("afconvert")
        // 32 kbps: the AAC encoder rejects higher bit rates for 16 kHz mono.
        .args(["-f", "m4af", "-d", "aac", "-b", "32000"])
        .arg(&wav)
        .arg(&m4a)
        .status()
        .is_ok_and(|s| s.success());
    if converted {
        let _ = fs::remove_file(&wav);
        Ok(format!("audio/{stem}.m4a"))
    } else {
        Ok(format!("audio/{stem}.wav"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dictate(dir: &Path, text: &str, new_note: Option<Option<String>>) -> String {
        let job = DictationJob {
            dir: dir.to_path_buf(),
            text: text.into(),
            at: Local.with_ymd_and_hms(2026, 10, 15, 14, 32, 0).unwrap(),
            source: "Wispr Flow",
            new_note,
            environment: Environment::default(),
        };
        match process_dictation(job).unwrap() {
            Outcome::Appended { note, .. } => note,
            _ => panic!("not appended"),
        }
    }

    #[test]
    fn new_notes_get_a_generic_title_not_the_first_sentence() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(dictate(tmp.path(), "The checkout button does nothing. Fix it.", None), "Untitled");
        assert_eq!(dictate(tmp.path(), "Another thing entirely.", Some(None)), "Untitled");
        assert_eq!(dictate(tmp.path(), "Something else.", Some(Some("Checkout".into()))), "Checkout");

        let notes = notes::list_notes(tmp.path());
        let titles: Vec<&str> = notes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, ["Untitled", "Untitled 2", "Checkout"]);
        let content = fs::read_to_string(notes[0].file()).unwrap();
        assert!(content.starts_with("---\ntitle: Untitled\n"), "{content}");
    }

    #[test]
    fn screen_entries_hold_screenshots_and_annotations() {
        let tmp = tempfile::tempdir().unwrap();
        let at = Local.with_ymd_and_hms(2026, 10, 15, 14, 40, 5).unwrap();
        let image = |offset: f32, details: Option<&str>| CapturedImage {
            offset,
            at,
            bytes: vec![0x89, b'P', b'N', b'G'],
            extension: "png".into(),
            details: details.map(Into::into),
        };
        let job = ScreenJob {
            dir: tmp.path().to_path_buf(),
            at,
            images: vec![image(0.0, Some("**Measured** in pt:\n- Area 10 × 10 at (0, 0)")), image(4.0, None)],
            texts: vec![(2.5, "### 1. button \"Pay\"\n**Feedback:** Bigger".into())],
            new_note: None,
            environment: Environment::default(),
            model: "m".into(),
        };
        assert!(matches!(process_screen(job).unwrap(), Outcome::Appended { number: 1, .. }));
        let note = notes::latest_note(tmp.path()).unwrap();
        let content = fs::read_to_string(note.file()).unwrap();
        assert!(
            content.ends_with(
                "## 1 · 14:40 · Screen\n\n![14:40:05](images/01-144005.png)\n\n**Measured** in pt:\n- Area 10 × 10 at \
                 (0, 0)\n\n### 1. button \"Pay\"\n**Feedback:** Bigger\n\n![14:40:05](images/01-144005-2.png)\n"
            ),
            "{content}"
        );
    }
}
