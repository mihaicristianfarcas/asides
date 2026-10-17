//! Turns one recording (transcript segments + attached images) into a Markdown entry.

/// A pause longer than this between two transcript segments starts a new paragraph.
const PARAGRAPH_GAP_SECS: f32 = 1.5;

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Seconds from the start of the recording.
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// Something added to a recording at a moment: a screenshot, text (a Measure & Annotate block),
/// or a screenshot followed by text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attachment {
    /// Seconds from the start of the recording at which it was attached.
    pub offset: f32,
    /// Path relative to the note folder, e.g. `images/03-143851.png`; empty for text alone.
    pub rel_path: String,
    /// Alt text, e.g. the wall-clock time `14:38:51`.
    pub label: String,
    /// Markdown shown after the image.
    pub text: Option<String>,
}

impl Attachment {
    pub fn markdown(&self) -> String {
        let image = (!self.rel_path.is_empty()).then(|| format!("![{}]({})", self.label, self.rel_path));
        let text = self.text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        image.into_iter().chain(text).collect::<Vec<_>>().join("\n\n")
    }
}

/// Drops the places in `texts` (`**In:** …`, each a text of its own: where the user was when they
/// started talking) that say where the last place before them already said, in the entry's order;
/// a block that names its place (an annotated screenshot's) says it too. Sorts `texts` by offset.
pub fn without_repeated_places(texts: &mut Vec<(f32, String)>) {
    texts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut last: Option<String> = None;
    texts.retain(|(_, text)| {
        let place = |line: &str| line.starts_with(crate::inspect::PLACE).then(|| line.to_string());
        if !text.contains('\n')
            && let Some(here) = place(text.trim())
        {
            return last.replace(here.clone()).is_none_or(|before| before != here);
        }
        if let Some(named) = text.lines().rev().find_map(|l| place(l.trim())) {
            last = Some(named);
        }
        true
    });
}

/// `## 3 · 14:38` followed by the verbatim transcript. Each image is placed before the first
/// segment that starts at or after the moment it was attached, splitting the paragraph there.
pub fn compose_entry(
    number: u32,
    time_label: &str,
    audio_rel_path: Option<&str>,
    segments: &[Segment],
    attachments: &[Attachment],
) -> String {
    let mut heading = format!("## {number} · {time_label}");
    if let Some(audio) = audio_rel_path {
        heading.push_str(&format!(" · [audio]({audio})"));
    }
    let mut blocks = Blocks::new(heading);
    let mut attachments: Vec<&Attachment> = attachments.iter().collect();
    attachments.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let mut pending = attachments.into_iter().peekable();

    let mut paragraph = String::new();
    let mut last_end: Option<f32> = None;
    for segment in segments {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        while let Some(att) = pending.next_if(|a| a.offset <= segment.start) {
            blocks.said(&mut paragraph);
            blocks.attach(att.markdown());
        }
        if last_end.is_some_and(|end| segment.start - end > PARAGRAPH_GAP_SECS) {
            blocks.said(&mut paragraph);
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(text);
        last_end = Some(segment.end);
    }
    blocks.said(&mut paragraph);
    for att in pending {
        blocks.attach(att.markdown());
    }
    blocks.list.join("\n\n")
}

/// `## 7 · 14:41 · Screen` followed by screenshots and text, in order: an entry made without
/// speaking, such as a Measure & Annotate session outside a recording.
pub fn compose_attachments_entry(number: u32, time_label: &str, source: &str, attachments: &[Attachment]) -> String {
    let mut blocks = Blocks::new(format!("## {number} · {time_label} · {source}"));
    for att in attachments {
        blocks.attach(att.markdown());
    }
    blocks.list.join("\n\n")
}

/// Measure & Annotate's blocks (and nothing said between them) as Markdown.
pub fn join_blocks(texts: impl IntoIterator<Item = String>) -> String {
    let mut blocks = Blocks { list: Vec::new(), attached: false };
    for text in texts {
        blocks.attach(text);
    }
    blocks.list.join("\n\n")
}

/// An entry's blocks, separated by blank lines. Measurements are list items: made one after
/// another, with nothing said between them, they stay one list.
struct Blocks {
    list: Vec<String>,
    /// The last block is an attachment, not what was said.
    attached: bool,
}

impl Blocks {
    fn new(heading: String) -> Blocks {
        Blocks { list: vec![heading], attached: false }
    }

    /// Ends the paragraph being said.
    fn said(&mut self, paragraph: &mut String) {
        if !paragraph.is_empty() {
            self.list.push(std::mem::take(paragraph));
            self.attached = false;
        }
    }

    fn attach(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let item = |s: &str| s.starts_with("- ");
        match self.list.last_mut() {
            Some(last) if self.attached && item(&text) && last.lines().last().is_some_and(item) => {
                last.push('\n');
                last.push_str(&text);
            }
            _ => self.list.push(text),
        }
        self.attached = true;
    }
}

/// `## 6 · 14:40 · Wispr Flow` followed by text another app transcribed, exactly as it came.
pub fn compose_dictation_entry(number: u32, time_label: &str, source: &str, text: &str) -> String {
    format!("## {number} · {time_label} · {source}\n\n{}", text.trim())
}

/// The plain transcript of a recording, used for titles and for "nothing heard" checks.
pub fn plain_text(segments: &[Segment]) -> String {
    segments.iter().map(|s| s.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, end: f32, text: &str) -> Segment {
        Segment { start, end, text: format!(" {text}") }
    }

    fn img(offset: f32, n: u32) -> Attachment {
        Attachment { offset, rel_path: format!("images/0{n}.png"), label: format!("img{n}"), text: None }
    }

    #[test]
    fn says_each_place_once() {
        let mut texts = vec![
            (5.0, "**In:** Safari · \"Shop\"".to_string()),
            (0.0, "**In:** Safari · \"Shop\"".to_string()),
            (2.0, "- Distance 12 across".to_string()),
            (8.0, "![shot](images/1.png)\n\n**In:** Simulator\n### 1. button".to_string()),
            (9.0, "**In:** Simulator".to_string()),
            (12.0, "**In:** Safari · \"Shop\"".to_string()),
        ];
        without_repeated_places(&mut texts);
        let offsets: Vec<f32> = texts.iter().map(|t| t.0).collect();
        assert_eq!(offsets, [0.0, 2.0, 8.0, 12.0], "{texts:?}");
    }

    #[test]
    fn joins_segments_and_breaks_paragraphs_on_long_pauses() {
        let segments = [seg(0.0, 2.0, "One."), seg(2.2, 4.0, "Two."), seg(7.0, 9.0, "Three.")];
        assert_eq!(compose_entry(1, "14:32", None, &segments, &[]), "## 1 · 14:32\n\nOne. Two.\n\nThree.");
    }

    #[test]
    fn places_images_by_timestamp() {
        let segments = [seg(0.0, 2.0, "Here."), seg(2.1, 4.0, "The padding."), seg(4.1, 6.0, "And this.")];
        let attachments = [img(5.0, 3), img(0.0, 1), img(2.05, 2), img(99.0, 4)];
        assert_eq!(
            compose_entry(5, "09:10", Some("audio/05.m4a"), &segments, &attachments),
            "## 5 · 09:10 · [audio](audio/05.m4a)\n\n![img1](images/01.png)\n\nHere.\n\n![img2](images/02.png)\n\nThe padding. And this.\n\n![img3](images/03.png)\n\n![img4](images/04.png)"
        );
    }

    #[test]
    fn measurements_in_a_row_are_one_list() {
        let text = |offset: f32, t: &str| Attachment { offset, text: Some(t.into()), ..Attachment::default() };
        let segments = [seg(0.0, 2.0, "- This one."), seg(5.0, 6.0, "And that.")];
        let attachments =
            [text(1.0, "- Area 1"), text(1.5, "- Area 2"), text(5.5, "- Area 3"), text(5.6, "### 1. row")];
        assert_eq!(
            compose_entry(1, "10:00", None, &segments, &attachments),
            "## 1 · 10:00\n\n- This one.\n\n- Area 1\n- Area 2\n\nAnd that.\n\n- Area 3\n\n### 1. row"
        );
        assert_eq!(
            join_blocks(["**Measured** in pt".into(), "- A".into(), "- B".into()]),
            "**Measured** in pt\n\n- A\n- B"
        );
    }

    #[test]
    fn images_only_entry() {
        assert_eq!(
            compose_entry(2, "10:00", None, &[seg(0.0, 1.0, "  ")], &[img(1.0, 1)]),
            "## 2 · 10:00\n\n![img1](images/01.png)"
        );
    }

    #[test]
    fn text_attachments_follow_speech_by_time() {
        let segments = [seg(0.0, 2.0, "This one."), seg(5.0, 7.0, "Too much padding.")];
        let screen = Attachment {
            offset: 1.0,
            rel_path: "images/01.png".into(),
            label: "14:00:01".into(),
            text: Some("**Measured** in pt:\n- Area 10 × 10 at (0, 0)".into()),
        };
        let note = Attachment { offset: 3.0, text: Some("### 1. button \"Save\"".into()), ..Attachment::default() };
        assert_eq!(
            compose_entry(1, "14:00", None, &segments, &[note.clone(), screen.clone()]),
            "## 1 · 14:00\n\nThis one.\n\n![14:00:01](images/01.png)\n\n**Measured** in pt:\n- Area 10 × 10 at (0, 0)\n\n\
             ### 1. button \"Save\"\n\nToo much padding."
        );
        assert_eq!(
            compose_attachments_entry(2, "14:01", "Screen", &[screen, note, Attachment::default()]),
            "## 2 · 14:01 · Screen\n\n![14:00:01](images/01.png)\n\n**Measured** in pt:\n- Area 10 × 10 at (0, 0)\n\n\
             ### 1. button \"Save\""
        );
    }

    #[test]
    fn dictation_entry_names_its_source() {
        assert_eq!(
            compose_dictation_entry(6, "14:40", "Wispr Flow", "\nThe card needs padding.\n\nAnd a border.\n"),
            "## 6 · 14:40 · Wispr Flow\n\nThe card needs padding.\n\nAnd a border."
        );
    }

    #[test]
    fn plain_text_trims_segments() {
        assert_eq!(plain_text(&[seg(0.0, 1.0, "A."), seg(1.0, 2.0, ""), seg(2.0, 3.0, "B.")]), "A. B.");
    }
}
