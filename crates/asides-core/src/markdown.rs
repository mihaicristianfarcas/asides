//! Turns what was said (Whisper's segments, with each word's time) and what was made meanwhile
//! (screenshots, pins, measurements, where the user was) into an aside's message, in the order it
//! happened: a pin leaves its number (`[2]`) after the word said as it was placed, and its details
//! follow that paragraph.

/// A pause longer than this between two transcript segments starts a new paragraph.
const PARAGRAPH_GAP_SECS: f32 = 1.5;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Segment {
    /// Seconds from the start of the recording.
    pub start: f32,
    pub end: f32,
    pub text: String,
    /// Its words, when Whisper timed them: they make `text`.
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    /// Seconds from the start of the recording.
    pub start: f32,
    pub text: String,
}

/// Something added to a recording at a moment: a screenshot, text (a pin, a measurement, where the
/// user was), or a screenshot followed by text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attachment {
    /// Seconds from the start of the recording at which it was attached.
    pub offset: f32,
    /// Path relative to the aside's folder, e.g. `images/1.png`; empty for text alone.
    pub rel_path: String,
    /// Alt text, e.g. `Screenshot 1`.
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

    /// What it is in the message: a pin (its number), a measurement, or a block of its own.
    fn kind(&self) -> Kind {
        let text = self.text.as_deref().unwrap_or_default().trim();
        if !self.rel_path.is_empty() || text.is_empty() {
            return Kind::Block;
        }
        if let Some(number) = pin_number(text) {
            return Kind::Pin(number);
        }
        if text.lines().all(|l| l.starts_with("- ")) { Kind::Measurement } else { Kind::Block }
    }
}

enum Kind {
    Pin(u32),
    Measurement,
    Block,
}

/// A pin's number, from its block's first line (`### 2. button "Save"`, or `2. **button**`).
pub fn pin_number(block: &str) -> Option<u32> {
    let line = block.lines().next()?;
    let rest = line.strip_prefix("### ").unwrap_or(line);
    let (number, after) = rest.split_once(". ")?;
    let heading = line.starts_with("### ") || after.starts_with("**");
    (heading && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())).then(|| number.parse().ok())?
}

/// An aside's message: the verbatim transcript with what was made while saying it, in order.
/// Screenshots and places go where they were made, splitting what was said there; a pin's number
/// goes after the word said as it was placed (in a pause, at the end or start of the paragraph
/// nearer in time), and its details, like measurements, after that paragraph. Nothing said: what
/// was made, in order.
pub fn compose_message(segments: &[Segment], attachments: &[Attachment]) -> String {
    let mut attachments: Vec<&Attachment> = attachments.iter().collect();
    attachments.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let words = spoken(segments);
    let mut blocks = Blocks::default();
    if words.is_empty() {
        for att in attachments {
            blocks.attach(att.markdown());
        }
        return blocks.list.join("\n\n");
    }
    let mut pending = attachments.into_iter().peekable();
    let mut paragraph = Paragraph::default();
    let mut last_end = f32::NEG_INFINITY;
    for word in &words {
        // A pause before this word ends the paragraph: what was made in it goes with the nearer.
        let mut due = word.breaks;
        while let Some(att) = pending.next_if(|a| a.offset <= word.start) {
            if due && att.offset - last_end > word.start - att.offset {
                blocks.end(&mut paragraph);
                due = false;
            }
            blocks.add(att, &mut paragraph);
        }
        if due {
            blocks.end(&mut paragraph);
        }
        paragraph.say(word.text);
        last_end = word.end;
    }
    for att in pending {
        blocks.add(att, &mut paragraph);
    }
    blocks.end(&mut paragraph);
    blocks.list.join("\n\n")
}

/// A word said, with whether a pause before it starts a paragraph.
struct Spoken<'a> {
    start: f32,
    end: f32,
    text: &'a str,
    breaks: bool,
}

/// The words said, in order: each segment's timed words, or the segment as one.
fn spoken(segments: &[Segment]) -> Vec<Spoken<'_>> {
    let mut words: Vec<Spoken> = Vec::new();
    for segment in segments.iter().filter(|s| !s.text.trim().is_empty()) {
        let breaks = words.last().is_some_and(|w| segment.start - w.end > PARAGRAPH_GAP_SECS);
        let timed: Vec<(f32, &str)> = match segment.words.as_slice() {
            [] => vec![(segment.start, segment.text.as_str())],
            list => list.iter().map(|w| (w.start, w.text.as_str())).collect(),
        };
        let mut first = true;
        for (i, &(start, text)) in timed.iter().enumerate() {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let end = timed.get(i + 1).map_or(segment.end, |next| next.0).max(start);
            words.push(Spoken { start, end, text, breaks: breaks && first });
            first = false;
        }
    }
    words
}

/// What's being said, with the details of what was placed while saying it.
#[derive(Default)]
struct Paragraph {
    text: String,
    notes: Vec<String>,
}

impl Paragraph {
    fn say(&mut self, word: &str) {
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        self.text.push_str(word);
    }

    /// `[number]` after what was said so far, before its closing punctuation.
    fn mark(&mut self, number: u32) {
        let cut = self.text.trim_end_matches([',', '.', ';', ':', '!', '?', '…']).len();
        let tail = self.text.split_off(cut);
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        self.text.push_str(&format!("[{number}]{tail}"));
    }
}

/// Measure & Annotate's blocks (and nothing said between them) as Markdown.
pub fn join_blocks(texts: impl IntoIterator<Item = String>) -> String {
    let mut blocks = Blocks::default();
    for text in texts {
        blocks.attach(text);
    }
    blocks.list.join("\n\n")
}

/// A message's blocks, separated by blank lines. Measurements are list items: made one after
/// another, with nothing said between them, they stay one list. A place (`**In:** …`) the last one
/// already said isn't said again.
#[derive(Default)]
struct Blocks {
    list: Vec<String>,
    /// The last block is an attachment, not what was said.
    attached: bool,
    place: Option<String>,
}

impl Blocks {
    /// `att`, made while `paragraph` was being said.
    fn add(&mut self, att: &Attachment, paragraph: &mut Paragraph) {
        match att.kind() {
            Kind::Pin(number) => {
                paragraph.mark(number);
                paragraph.notes.extend(att.text.clone());
            }
            Kind::Measurement => paragraph.notes.extend(att.text.clone()),
            Kind::Block => {
                self.end(paragraph);
                self.attach(att.markdown());
            }
        }
    }

    /// Ends the paragraph being said, its pins' details after it.
    fn end(&mut self, paragraph: &mut Paragraph) {
        let Paragraph { text, notes } = std::mem::take(paragraph);
        if !text.is_empty() {
            self.list.push(text);
            self.attached = false;
        }
        for note in notes {
            self.attach(note);
        }
    }

    fn attach(&mut self, text: String) {
        let place = |line: &str| line.trim().starts_with(crate::inspect::PLACE);
        let mut lines: Vec<&str> = Vec::new();
        for line in text.trim().lines() {
            if place(line) {
                if self.place.as_deref() == Some(line.trim()) {
                    continue;
                }
                self.place = Some(line.trim().to_string());
            }
            lines.push(line);
        }
        let text = lines.join("\n").trim().to_string();
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

/// A message as it's typed into an agent's session: text, and screenshots (their paths) where they
/// were made.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Image(String),
}

/// Splits a message into its text and its images (lines that are only `![label](path)`), in order.
pub fn message_parts(message: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text: Vec<&str> = Vec::new();
    let flush = |text: &mut Vec<&str>, parts: &mut Vec<Part>| {
        let joined = text.join("\n");
        let joined = joined.trim_matches('\n');
        if !joined.trim().is_empty() {
            parts.push(Part::Text(joined.to_string()));
        }
        text.clear();
    };
    for line in message.lines() {
        let image = line
            .trim()
            .strip_prefix("![")
            .and_then(|rest| rest.split_once("]("))
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .filter(|path| !path.is_empty() && !path.contains(' '));
        match image {
            Some(path) => {
                flush(&mut text, &mut parts);
                parts.push(Part::Image(path.to_string()));
            }
            None => text.push(line),
        }
    }
    flush(&mut text, &mut parts);
    parts
}

/// The plain transcript of a recording, used for titles and for "nothing heard" checks.
pub fn plain_text(segments: &[Segment]) -> String {
    segments.iter().map(|s| s.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, end: f32, text: &str) -> Segment {
        Segment { start, end, text: format!(" {text}"), words: Vec::new() }
    }

    /// A segment whose words start a second apart from `start`.
    fn timed(start: f32, text: &str) -> Segment {
        let words: Vec<Word> =
            text.split(' ').enumerate().map(|(i, w)| Word { start: start + i as f32, text: format!(" {w}") }).collect();
        let end = start + words.len() as f32;
        Segment { start, end, text: format!(" {text}"), words }
    }

    fn img(offset: f32, n: u32) -> Attachment {
        Attachment { offset, rel_path: format!("images/{n}.png"), label: format!("Screenshot {n}"), text: None }
    }

    fn text(offset: f32, text: &str) -> Attachment {
        Attachment { offset, text: Some(text.into()), ..Attachment::default() }
    }

    #[test]
    fn joins_segments_and_breaks_paragraphs_on_long_pauses() {
        let segments = [seg(0.0, 2.0, "One."), seg(2.2, 4.0, "Two."), seg(7.0, 9.0, "Three.")];
        assert_eq!(compose_message(&segments, &[]), "One. Two.\n\nThree.");
    }

    #[test]
    fn places_images_by_timestamp() {
        let segments = [seg(0.0, 2.0, "Here."), seg(2.1, 4.0, "The padding."), seg(4.1, 6.0, "And this.")];
        let attachments = [img(5.0, 3), img(0.0, 1), img(2.05, 2), img(99.0, 4)];
        assert_eq!(
            compose_message(&segments, &attachments),
            "![Screenshot 1](images/1.png)\n\nHere.\n\n![Screenshot 2](images/2.png)\n\nThe padding. And this.\n\n\
             ![Screenshot 3](images/3.png)\n\n![Screenshot 4](images/4.png)"
        );
    }

    #[test]
    fn pins_are_numbered_where_they_were_said() {
        // "this" at 2 s, "here" at 7 s and 9 s; the pins just after each.
        let segments = [timed(0.0, "I'd make this a different color, and here and here, moved down.")];
        let attachments = [
            text(9.3, "### 3. button \"Docs\""),
            text(2.5, "### 1. link \"Version\"\n**Intent:** change"),
            text(7.2, "### 2. button \"Download\""),
            text(7.5, "- Area 10 × 10 at (0, 0)"),
            img(0.0, 1),
        ];
        assert_eq!(
            compose_message(&segments, &attachments),
            "![Screenshot 1](images/1.png)\n\n\
             I'd make this [1] a different color, and here [2] and here [3], moved down.\n\n\
             ### 1. link \"Version\"\n**Intent:** change\n\n### 2. button \"Download\"\n\n\
             - Area 10 × 10 at (0, 0)\n\n### 3. button \"Docs\""
        );
    }

    #[test]
    fn a_pin_in_a_pause_goes_with_the_nearer_paragraph() {
        let segments = [timed(0.0, "First."), timed(10.0, "Second.")];
        let near_first = [text(2.0, "### 1. a")];
        assert_eq!(compose_message(&segments, &near_first), "First [1].\n\n### 1. a\n\nSecond.");
        let near_second = [text(9.0, "### 1. a")];
        assert_eq!(compose_message(&segments, &near_second), "First.\n\n[1] Second.\n\n### 1. a");
        // A screenshot in the pause splits them.
        assert_eq!(
            compose_message(&segments, &[img(9.0, 1), text(9.5, "### 1. a")]),
            "First.\n\n![Screenshot 1](images/1.png)\n\n[1] Second.\n\n### 1. a"
        );
    }

    #[test]
    fn says_each_place_once() {
        let segments = [seg(1.0, 2.0, "Here."), seg(9.0, 10.0, "There.")];
        let shot = Attachment { text: Some("**In:** Safari (\"Shop\")\n**Measured** in pt".into()), ..img(0.5, 1) };
        let attachments =
            [text(0.0, "**In:** Safari (\"Shop\")"), shot, text(8.0, "**In:** Notes"), text(12.0, "**In:** Notes")];
        assert_eq!(
            compose_message(&segments, &attachments),
            "**In:** Safari (\"Shop\")\n\n![Screenshot 1](images/1.png)\n\n**Measured** in pt\n\nHere.\n\n\
             **In:** Notes\n\nThere."
        );
    }

    #[test]
    fn measurements_in_a_row_are_one_list() {
        let segments = [seg(0.0, 2.0, "- This one."), seg(5.0, 6.0, "And that.")];
        let attachments =
            [text(1.0, "- Area 1"), text(1.5, "- Area 2"), text(5.5, "- Area 3"), text(5.6, "### 1. row")];
        assert_eq!(
            compose_message(&segments, &attachments),
            "- This one.\n\n- Area 1\n- Area 2\n\nAnd that [1].\n\n- Area 3\n\n### 1. row"
        );
        assert_eq!(
            join_blocks(["**Measured** in pt".into(), "- A".into(), "- B".into()]),
            "**Measured** in pt\n\n- A\n- B"
        );
    }

    #[test]
    fn nothing_said_is_what_was_made_in_order() {
        let attachments = [text(2.0, "### 1. row"), img(1.0, 1)];
        assert_eq!(
            compose_message(&[seg(0.0, 1.0, "  ")], &attachments),
            "![Screenshot 1](images/1.png)\n\n### 1. row"
        );
    }

    #[test]
    fn reads_pin_numbers() {
        assert_eq!(pin_number("### 12. button \"Save\"\n**Intent:** fix"), Some(12));
        assert_eq!(pin_number("3. **button** (Safari)"), Some(3));
        assert_eq!(pin_number("- Area 1"), None);
        assert_eq!(pin_number("1. Not a pin"), None);
    }

    #[test]
    fn messages_split_into_text_and_images() {
        let segments = [seg(0.0, 2.0, "This one."), seg(5.0, 7.0, "Too much padding.")];
        let message = compose_message(&segments, &[img(3.0, 1)]);
        assert_eq!(message, "This one.\n\n![Screenshot 1](images/1.png)\n\nToo much padding.");
        assert_eq!(
            message_parts(&message),
            [
                Part::Text("This one.".into()),
                Part::Image("images/1.png".into()),
                Part::Text("Too much padding.".into())
            ]
        );
        assert_eq!(
            message_parts("![a](x.png)\n![b](y.png)"),
            [Part::Image("x.png".into()), Part::Image("y.png".into())]
        );
        assert_eq!(message_parts("Look at ![this](x.png) here"), [Part::Text("Look at ![this](x.png) here".into())]);
    }

    #[test]
    fn plain_text_trims_segments() {
        assert_eq!(plain_text(&[seg(0.0, 1.0, "A."), seg(1.0, 2.0, ""), seg(2.0, 3.0, "B.")]), "A. B.");
    }
}
