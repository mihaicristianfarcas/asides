//! What's said during a Measure & Annotate session comes in takes (each press of the talk key, the
//! microphone or the Asides shortcut held). When the session is done they're joined into one recording,
//! so what was said and what was annotated make one aside: the takes in order with a pause between
//! them (each its own paragraph), and each moment of the session placed on that recording, so a
//! annotation sits among what was said when it was made.

use crate::markdown::Segment;
use crate::resample::resample;

/// The silence between two takes, longer than a paragraph break in the transcript.
pub const PAUSE_SECS: f32 = 2.0;

/// One take: when it started, in seconds into the session (negative when it was under way before
/// the session opened), and what it recorded.
pub struct Take<'a> {
    pub start: f32,
    pub samples: &'a [f32],
    pub sample_rate: u32,
}

/// The takes as one recording, at the first take's rate.
pub struct Joined {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Each take, in order: (its start in the session, its start in the recording, its length),
    /// in seconds.
    spans: Vec<(f32, f32, f32)>,
}

pub fn join(takes: &[Take]) -> Joined {
    let mut order: Vec<&Take> = takes.iter().collect();
    order.sort_by(|a, b| a.start.total_cmp(&b.start));
    let sample_rate = order.first().map_or(16_000, |t| t.sample_rate.max(1));
    let rate = sample_rate as f32;
    let mut samples: Vec<f32> = Vec::new();
    let mut spans = Vec::new();
    for (i, take) in order.into_iter().enumerate() {
        if i > 0 {
            samples.resize(samples.len() + (PAUSE_SECS * rate) as usize, 0.0);
        }
        let audio = samples.len() as f32 / rate;
        let take_samples = resample(take.samples, take.sample_rate.max(1), sample_rate);
        spans.push((take.start, audio, take_samples.len() as f32 / rate));
        samples.extend(take_samples);
    }
    Joined { samples, sample_rate, spans }
}

impl Joined {
    /// Where a moment of the session (seconds into it) falls in the recording: in a take, at the
    /// same point of it; between two, along the pause between them, in order; before the first or
    /// after the last, as far before or after.
    pub fn offset(&self, time: f32) -> f32 {
        let Some(&(first, ..)) = self.spans.first() else { return time };
        if time < first {
            return time - first;
        }
        // Where the previous take ended, in the session and in the recording.
        let mut previous = (first, 0.0);
        for &(start, audio, length) in &self.spans {
            if time < start {
                let (end, audio_end) = previous;
                let along = ((time - end) / (start - end)).clamp(0.0, 1.0);
                return audio_end + along * (audio - audio_end);
            }
            if time <= start + length {
                return audio + (time - start);
            }
            previous = (start + length, audio + length);
        }
        previous.1 + (time - previous.0)
    }

    /// Where each take is in the recording: its start and length, in seconds.
    pub fn takes(&self) -> Vec<(f32, f32)> {
        self.spans.iter().map(|&(_, audio, length)| (audio, length)).collect()
    }
}

/// How far Whisper's times may run past a take (seconds) before a segment counts as running into
/// the next one.
const SLACK_SECS: f32 = 0.5;

/// Whisper's `segments` of takes joined into one recording (`takes`: where each is, start and
/// length), each kept within the take it's most in. Whisper runs its times through the pause
/// between two takes, so the second's first words seemed to start before it, and before what was
/// added as it began (where the user was). `None` when a segment runs from one take well into the
/// next: Whisper heard the two as one, and each has to be transcribed on its own.
pub fn fit_segments(segments: Vec<Segment>, takes: &[(f32, f32)]) -> Option<Vec<Segment>> {
    let crosses =
        |s: &Segment| takes.windows(2).any(|w| s.start < w[0].0 + w[0].1 - SLACK_SECS && s.end > w[1].0 + SLACK_SECS);
    if segments.iter().any(crosses) {
        return None;
    }
    let overlap = |s: &Segment, &(start, length): &(f32, f32)| s.end.min(start + length) - s.start.max(start);
    Some(
        segments
            .into_iter()
            .map(|s| match takes.iter().max_by(|a, b| overlap(&s, a).total_cmp(&overlap(&s, b))) {
                Some(&(start, length)) => {
                    let end = start + length;
                    Segment { start: s.start.clamp(start, end), end: s.end.clamp(start, end), ..s }
                }
                None => s,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{Attachment, compose_entry};

    #[test]
    fn joins_takes_with_a_pause_and_places_moments() {
        let (a, b) = (vec![0.5; 32_000], vec![0.25; 8_000]);
        // Out of order, and the second at another rate.
        let joined = join(&[
            Take { start: 10.0, samples: &b, sample_rate: 8_000 },
            Take { start: 1.0, samples: &a, sample_rate: 16_000 },
        ]);
        assert_eq!(joined.sample_rate, 16_000);
        // 2 s, the pause, then 1 s.
        assert_eq!(joined.samples.len(), 16_000 * 5);
        assert_eq!((joined.samples[0], joined.samples[16_000 * 3]), (0.5, 0.0));
        assert!((joined.samples[16_000 * 9 / 2] - 0.25).abs() < 0.01);
        let at = |t: f32| joined.offset(t);
        // Before the first take, in it, at its end, along the pause, in the second, after it.
        assert_eq!([at(0.0), at(2.0), at(3.0), at(6.5), at(10.5), at(12.0)], [-1.0, 1.0, 2.0, 3.0, 4.5, 6.0]);
        assert!(at(3.5) < at(6.5) && at(6.5) < at(9.9) && at(9.9) < at(10.0));
        assert_eq!(joined.takes(), [(0.0, 2.0), (4.0, 1.0)]);
    }

    fn said(start: f32, end: f32, text: &str) -> Segment {
        Segment { start, end, text: text.into() }
    }

    #[test]
    fn keeps_what_was_said_in_its_take() {
        // Two takes, 0–4 s and 6–9 s: Whisper's times run through the pause between them.
        let takes = [(0.0, 4.0), (6.0, 3.0)];
        let fitted = fit_segments(vec![said(0.0, 4.8, "One."), said(4.8, 9.0, "Two.")], &takes).unwrap();
        let times: Vec<(f32, f32)> = fitted.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(times, [(0.0, 4.0), (6.0, 9.0)]);
        // Where the user was when the second began goes before what they said in it.
        let place = Attachment { offset: 6.0, text: Some("**In:** Calculator".into()), ..Attachment::default() };
        let entry = compose_entry(1, "10:21", None, &fitted, &[place]);
        assert_eq!(entry, "## 1 · 10:21\n\nOne.\n\n**In:** Calculator\n\nTwo.");
        // One segment across both: they have to be transcribed apart.
        assert_eq!(fit_segments(vec![said(1.0, 8.0, "x")], &takes), None);
        assert!(fit_segments(vec![said(0.0, 4.3, "x"), said(5.8, 9.0, "y")], &takes).is_some());
    }

    #[test]
    fn nothing_said() {
        let joined = join(&[]);
        assert!(joined.samples.is_empty());
        assert_eq!(joined.offset(4.0), 4.0);
    }
}
