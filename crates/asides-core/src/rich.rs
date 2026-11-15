//! An aside as the clipboard holds it, for one paste anywhere: its Markdown with the screenshots'
//! paths (what a terminal pastes, for an agent), and rich text (what a chat, a mail or a note
//! pastes): its paragraphs, with the little Markdown asides writes (`### ` headings, `**bold**`,
//! `` `code` ``) as styles, and its screenshots where they were made. Each system's clipboard puts
//! [`Rich::html`] there; the Mac's also draws the blocks as RTFD, and adds each screenshot as an
//! image of its own (what a chat attaches).

use std::fs;

use base64::Engine;

use crate::history::Aside;
use crate::markdown::Part;

/// An aside, ready for the clipboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Rich {
    /// Its Markdown, screenshots by their paths (as `Aside::markdown`).
    pub text: String,
    pub blocks: Vec<Block>,
    /// Its screenshots, in order.
    pub images: Vec<Shot>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A paragraph: its lines, each a run of spans.
    Text(Vec<Vec<Span>>),
    /// A screenshot (its index in `Rich::images`).
    Image(usize),
}

/// A run of text and how it looks.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub code: bool,
}

/// A screenshot's bytes and type.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub bytes: Vec<u8>,
    /// `image/png`, `image/jpeg`.
    pub mime: &'static str,
    /// `png`, `jpg`.
    pub extension: &'static str,
}

impl Rich {
    /// Aside `aside`, its screenshots read from its folder (one that's gone is left out).
    pub fn of(aside: &Aside) -> Rich {
        Rich::from_parts(&aside.parts(), |path| fs::read(path).ok())
    }

    /// From a message's parts, screenshots by path, `read` giving each one's bytes.
    pub fn from_parts(parts: &[Part], read: impl Fn(&str) -> Option<Vec<u8>>) -> Rich {
        let mut rich = Rich { text: crate::history::markdown_of(parts), blocks: Vec::new(), images: Vec::new() };
        for part in parts {
            match part {
                Part::Text(text) => rich.blocks.extend(paragraphs(text)),
                Part::Image(path) => {
                    let Some((mime, extension)) = image_type(path) else { continue };
                    let Some(bytes) = read(path) else { continue };
                    rich.blocks.push(Block::Image(rich.images.len()));
                    rich.images.push(Shot { bytes, mime, extension });
                }
            }
        }
        rich
    }

    /// As HTML, its screenshots in it (`data:` URLs, so it needs nothing beside it).
    pub fn html(&self) -> String {
        let mut html = String::from("<meta charset=\"utf-8\">");
        for block in &self.blocks {
            match block {
                Block::Text(lines) => {
                    html.push_str("<p>");
                    for (i, line) in lines.iter().enumerate() {
                        if i > 0 {
                            html.push_str("<br>");
                        }
                        for span in line {
                            let text = escape(&span.text);
                            let text = if span.code { format!("<code>{text}</code>") } else { text };
                            html.push_str(&if span.bold { format!("<b>{text}</b>") } else { text });
                        }
                    }
                    html.push_str("</p>");
                }
                Block::Image(n) => {
                    let shot = &self.images[*n];
                    let data = base64::engine::general_purpose::STANDARD.encode(&shot.bytes);
                    html.push_str(&format!(
                        "<p><img src=\"data:{};base64,{data}\" alt=\"Screenshot {}\"></p>",
                        shot.mime,
                        n + 1
                    ));
                }
            }
        }
        html
    }
}

/// What kind of image a screenshot's path names, by its extension.
fn image_type(path: &str) -> Option<(&'static str, &'static str)> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some(("image/png", "png")),
        "jpg" | "jpeg" => Some(("image/jpeg", "jpg")),
        _ => None,
    }
}

/// A message's text as paragraphs (split by blank lines), a heading's line all bold.
fn paragraphs(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = Vec::new();
    for line in text.lines().chain([""]) {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if !lines.is_empty() {
                blocks.push(Block::Text(std::mem::take(&mut lines)));
            }
            continue;
        }
        let heading = line.trim_start_matches('#');
        match heading.strip_prefix(' ').filter(|_| (1..=6).contains(&(line.len() - heading.len()))) {
            Some(title) => lines.push(spans(title.trim(), true)),
            None => lines.push(spans(line, false)),
        }
    }
    blocks
}

/// A line's spans: `**` turns bold on and off (when it's closed later), backticks quote code.
fn spans(line: &str, bold: bool) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut push = |text: &str, bold: bool, code: bool| {
        if text.is_empty() {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.bold == bold && last.code == code => last.text.push_str(text),
            _ => spans.push(Span { text: text.to_string(), bold, code }),
        }
    };
    let mut strong = false;
    let mut rest = line;
    while let Some(at) = rest.find(['*', '`']) {
        let (before, marked) = rest.split_at(at);
        if let Some(after) = marked.strip_prefix("**") {
            push(before, bold || strong, false);
            if strong || after.contains("**") {
                strong = !strong;
            } else {
                push("**", bold, false);
            }
            rest = after;
        } else if let Some((code, after)) = marked.strip_prefix('`').and_then(|m| m.split_once('`')) {
            push(before, bold || strong, false);
            push(code, bold || strong, true);
            rest = after;
        } else {
            push(before, bold || strong, false);
            push(&marked[..1], bold || strong, false);
            rest = &marked[1..];
        }
    }
    push(rest, bold || strong, false);
    spans
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, bold: bool, code: bool) -> Span {
        Span { text: text.into(), bold, code }
    }

    #[test]
    fn styles_what_asides_writes() {
        assert_eq!(spans("plain", false), [span("plain", false, false)]);
        assert_eq!(
            spans("**In:** Safari (`#save`)", false),
            [
                span("In:", true, false),
                span(" Safari (", false, false),
                span("#save", false, true),
                span(")", false, false)
            ]
        );
        // Unclosed markers stay as they were.
        assert_eq!(spans("2 ** 3 and a ` tick", false), [span("2 ** 3 and a ` tick", false, false)]);
        assert_eq!(spans("a*b", false), [span("a*b", false, false)]);
        assert_eq!(spans("bold **all", true), [span("bold **all", true, false)]);
    }

    #[test]
    fn splits_paragraphs_and_headings() {
        let blocks = paragraphs("First line\nsecond line\n\n### 1. button \"Save\"\n**Feedback:** Move it.\n\n\n");
        assert_eq!(
            blocks,
            [
                Block::Text(vec![vec![span("First line", false, false)], vec![span("second line", false, false)]]),
                Block::Text(vec![
                    vec![span("1. button \"Save\"", true, false)],
                    vec![span("Feedback:", true, false), span(" Move it.", false, false)],
                ]),
            ]
        );
        // Not a heading: no space, or too many.
        assert_eq!(paragraphs("#hashtag"), [Block::Text(vec![vec![span("#hashtag", false, false)]])]);
        assert_eq!(paragraphs("####### x"), [Block::Text(vec![vec![span("####### x", false, false)]])]);
    }

    #[test]
    fn keeps_screenshots_where_they_were_made() {
        let parts = vec![
            Part::Text("The Save button [1] sits low.".into()),
            Part::Image("/h/a/images/1.png".into()),
            Part::Image("/h/a/images/gone.png".into()),
            Part::Image("/h/a/notes.txt".into()),
            Part::Text("And <this> & that.".into()),
        ];
        let rich = Rich::from_parts(&parts, |path| (!path.contains("gone")).then(|| vec![1, 2, 3]));
        assert_eq!(rich.images, [Shot { bytes: vec![1, 2, 3], mime: "image/png", extension: "png" }]);
        assert_eq!(
            rich.blocks,
            [
                Block::Text(vec![vec![span("The Save button [1] sits low.", false, false)]]),
                Block::Image(0),
                Block::Text(vec![vec![span("And <this> & that.", false, false)]]),
            ]
        );
        // The text is the Markdown a terminal takes, every screenshot by its path.
        assert_eq!(
            rich.text,
            "The Save button [1] sits low.\n\n![Screenshot](/h/a/images/1.png)\n\n![Screenshot](/h/a/images/gone.png)\n\n\
             ![Screenshot](/h/a/notes.txt)\n\nAnd <this> & that."
        );
        assert_eq!(
            rich.html(),
            "<meta charset=\"utf-8\"><p>The Save button [1] sits low.</p>\
             <p><img src=\"data:image/png;base64,AQID\" alt=\"Screenshot 1\"></p><p>And &lt;this&gt; &amp; that.</p>"
        );
    }
}
