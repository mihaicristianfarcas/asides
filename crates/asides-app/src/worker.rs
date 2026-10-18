//! Background thread that transcribes what was said and keeps each aside in the history
//! (`~/.asides/history`), one job at a time and in order. The main thread sends it on.

use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use anyhow::{Context, Result};
use asides_core::helper::Target;
use asides_core::history::{self, Aside, Image};
use asides_core::markdown::{self, Attachment};
use asides_core::resample::resample;
use chrono::{DateTime, Local};

use crate::stt::{self, ModelManager, Transcriber};

const WHISPER_RATE: u32 = 16_000;
/// Unload the model after this long without work, to give the memory back.
const IDLE_UNLOAD: Duration = Duration::from_secs(15 * 60);

pub struct CapturedImage {
    /// Seconds since what was said started.
    pub offset: f32,
    pub bytes: Vec<u8>,
    pub extension: String,
    /// Markdown shown after the image (what was measured and annotated on it).
    pub details: Option<String>,
}

/// An aside to keep: what was said (maybe nothing), with what was made while saying it.
pub struct AsideJob {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub started: DateTime<Local>,
    pub images: Vec<CapturedImage>,
    /// Text added at a moment of it (an annotation, a measurement, where the user was), as
    /// (seconds, Markdown).
    pub texts: Vec<(f32, String)>,
    pub keep_audio: bool,
    pub model: String,
    pub languages: Vec<String>,
    /// Where each take is in `samples` (start and length, in seconds) when they're several takes
    /// joined: what's said in each is kept in it (`takes::fit_segments`).
    pub takes: Vec<(f32, f32)>,
    /// Where it goes, once kept.
    pub target: Option<Target>,
}

/// What the user said about one annotation (its card's microphone): transcribed straight away,
/// for the overlay to show and the aside to have.
pub struct TranscribeJob {
    pub id: u64,
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub model: String,
    pub languages: Vec<String>,
}

pub enum Job {
    /// Load this speech model now, while the user talks, if it's on disk (nothing comes back): the
    /// aside that follows doesn't wait for it after a while unused.
    Warm(String),
    Aside(Box<AsideJob>),
    Transcribe(Box<TranscribeJob>),
}

pub enum Outcome {
    /// Kept in the history; the main thread sends it to `target`.
    Kept {
        aside: Box<Aside>,
        target: Option<Target>,
    },
    NothingHeard,
    Failed(String),
    /// A `TranscribeJob`'s transcript (empty when nothing was heard), or why there's none.
    Transcribed {
        id: u64,
        text: Result<String, String>,
    },
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
                    Job::Warm(model) => {
                        warm(&model, &mut transcriber);
                        continue;
                    }
                    Job::Aside(job) => keep(*job, &models, &mut transcriber, &history::dir()),
                    Job::Transcribe(job) => Ok(Outcome::Transcribed {
                        id: job.id,
                        text: transcribe(
                            &resample(&job.samples, job.sample_rate, WHISPER_RATE),
                            &job.model,
                            &job.languages,
                            &models,
                            &mut transcriber,
                        )
                        .map(|segments| markdown::plain_text(&segments))
                        .map_err(|err| format!("{err:#}")),
                    }),
                };
                done(outcome.unwrap_or_else(|err| Outcome::Failed(format!("{err:#}"))));
            }
        })
        .expect("spawn worker thread");
    tx
}

/// Transcribes what was said and keeps the aside in `history`.
fn keep(
    job: AsideJob,
    models: &Arc<ModelManager>,
    transcriber: &mut Option<Transcriber>,
    history: &std::path::Path,
) -> Result<Outcome> {
    let pcm = resample(&job.samples, job.sample_rate.max(1), WHISPER_RATE);
    let nothing_else = job.images.is_empty() && job.texts.is_empty();
    if !pcm.is_empty() && nothing_else && pcm.iter().all(|s| *s == 0.0) {
        return Ok(Outcome::Failed(
            "The microphone delivered only silence. Check System Settings → Privacy & Security → Microphone.".into(),
        ));
    }
    let segments = match job.takes.as_slice() {
        _ if pcm.is_empty() => Vec::new(),
        [] | [_] => transcribe(&pcm, &job.model, &job.languages, models, transcriber)?,
        takes => {
            let whole = transcribe(&pcm, &job.model, &job.languages, models, transcriber)?;
            match asides_core::takes::fit_segments(whole, takes) {
                Some(segments) => segments,
                None => {
                    let mut segments = Vec::new();
                    for &(start, length) in takes {
                        let at = |seconds: f32| ((seconds * WHISPER_RATE as f32) as usize).min(pcm.len());
                        let take = &pcm[at(start)..at(start + length)];
                        let said = transcribe(take, &job.model, &job.languages, models, transcriber)?;
                        segments.extend(said.into_iter().map(|s| markdown::Segment {
                            start: s.start + start,
                            end: s.end + start,
                            ..s
                        }));
                    }
                    segments
                }
            }
        }
    };
    if markdown::plain_text(&segments).is_empty() && nothing_else {
        return Ok(Outcome::NothingHeard);
    }
    let mut attachments: Vec<Attachment> = job
        .images
        .iter()
        .enumerate()
        .map(|(i, image)| Attachment {
            offset: image.offset,
            rel_path: format!("{{image:{i}}}"),
            label: format!("Screenshot {}", i + 1),
            text: image.details.clone(),
        })
        .collect();
    attachments.extend(job.texts.iter().map(|(offset, text)| Attachment {
        offset: *offset,
        text: Some(text.clone()),
        ..Attachment::default()
    }));
    let message = markdown::compose_message(&segments, &attachments);
    let images: Vec<Image> = job.images.iter().map(|i| Image { bytes: &i.bytes, extension: &i.extension }).collect();
    let audio = if job.keep_audio && !pcm.is_empty() { encode_audio(&pcm).ok() } else { None };
    let aside = history::create(history, job.started, &message, &images, audio.as_ref().map(|(b, e)| (&b[..], *e)))
        .context("could not keep the aside")?;
    Ok(Outcome::Kept { aside: Box::new(aside), target: job.target })
}

/// Loads `model` unless it's loaded, or not on disk.
fn warm(model: &str, transcriber: &mut Option<Transcriber>) {
    if transcriber.as_ref().is_some_and(|t| t.model_id == model) {
        return;
    }
    let path = asides_core::models::ModelSpec::find(model).path();
    if path.is_file() {
        *transcriber = Transcriber::load(model, &path).ok();
    }
}

/// Whisper's segments for 16 kHz audio (none when it's silent), loading the model if needed.
fn transcribe(
    pcm: &[f32],
    model: &str,
    languages: &[String],
    models: &Arc<ModelManager>,
    transcriber: &mut Option<Transcriber>,
) -> Result<Vec<markdown::Segment>> {
    if stt::is_silent(pcm) {
        return Ok(Vec::new());
    }
    if transcriber.as_ref().is_none_or(|t| t.model_id != model) {
        *transcriber = None;
        let path = models.wait_ready(model)?;
        *transcriber = Some(Transcriber::load(model, &path)?);
    }
    transcriber.as_mut().unwrap().transcribe(pcm, languages)
}

/// The recording as AAC (the system's `afconvert`), or else WAV: its bytes and extension.
fn encode_audio(pcm: &[f32]) -> Result<(Vec<u8>, &'static str)> {
    let dir = tempfile_dir()?;
    let wav = dir.join("aside.wav");
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
    let m4a = dir.join("aside.m4a");
    let converted = Command::new("afconvert")
        // 32 kbps: the AAC encoder rejects higher bit rates for 16 kHz mono.
        .args(["-f", "m4af", "-d", "aac", "-b", "32000"])
        .arg(&wav)
        .arg(&m4a)
        .status()
        .is_ok_and(|s| s.success());
    let result = if converted { (fs::read(&m4a)?, "m4a") } else { (fs::read(&wav)?, "wav") };
    let _ = fs::remove_dir_all(&dir);
    Ok(result)
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("asides-audio-{}-{}", std::process::id(), history::now_ms()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn keeps_asides_made_without_speaking() {
        let tmp = tempfile::tempdir().unwrap();
        let at = Local.with_ymd_and_hms(2026, 10, 15, 14, 40, 5).unwrap();
        let image = |offset: f32, details: Option<&str>| CapturedImage {
            offset,
            bytes: vec![0x89, b'P', b'N', b'G'],
            extension: "png".into(),
            details: details.map(Into::into),
        };
        let job = AsideJob {
            samples: Vec::new(),
            sample_rate: 16_000,
            started: at,
            images: vec![image(0.0, Some("**Measured** in pt:\n- Area 10 × 10 at (0, 0)")), image(4.0, None)],
            texts: vec![(2.5, "### 1. button \"Pay\"\n**Feedback:** Bigger".into())],
            keep_audio: false,
            model: "m".into(),
            languages: Vec::new(),
            takes: Vec::new(),
            target: Some(Target::History),
        };
        let models = ModelManager::new(|_, _| {});
        let Outcome::Kept { aside, target } = keep(job, &models, &mut None, tmp.path()).unwrap() else {
            panic!("not kept")
        };
        assert_eq!(target, Some(Target::History));
        assert_eq!(aside.record.images, ["images/1.png", "images/2.png"]);
        assert_eq!(
            aside.message(),
            "![Screenshot 1](images/1.png)\n\n**Measured** in pt:\n- Area 10 × 10 at (0, 0)\n\n### 1. button \"Pay\"\n\
             **Feedback:** Bigger\n\n![Screenshot 2](images/2.png)"
        );
        assert_eq!(aside.record.title, "**Measured** in pt:");
    }
}
