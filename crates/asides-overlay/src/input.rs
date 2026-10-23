//! A text field for an annotation's typed feedback, after GPUI's input example: typing (with the
//! input methods macOS offers), selection, copy and paste, and deleting as macOS fields do (⌥⌫ a
//! word, ⌘⌫ to the start of the line; with fn, forward). Long text wraps, and the field grows a
//! line at a time to show it, up to `MAX_LINES`; past that it scrolls, keeping the caret in view.
//! One paragraph: Return saves the card (or, in the asides window, the setting). Its colors are a
//! [`Look`].
//!
//! An area ([`TextInput::area`]) takes lines (an aside's Markdown, in History): Return breaks the
//! line, the arrows go up and down, Home and End to the line's ends, and ⌘Z undoes.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hsla, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollWheelEvent, SharedString, Style,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill, hsla, point,
    prelude::*, px, relative, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    feedback_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToStart,
        DeleteToEnd,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Newline,
        Up,
        Down,
        SelectUp,
        SelectDown,
        Top,
        Bottom,
        Undo,
        Redo
    ]
);

pub const CONTEXT: &str = "FeedbackInput";
/// An area's: its keys go further.
pub const AREA_CONTEXT: &str = "TextArea";
/// The field grows to this many lines, then scrolls.
const MAX_LINES: usize = 8;
/// Typing within this of the last change undoes with it.
const UNDO_GROUP: Duration = Duration::from_millis(600);

pub fn bind_keys(cx: &mut App) {
    for context in [CONTEXT, AREA_CONTEXT] {
        let context = Some(context);
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, context),
            KeyBinding::new("delete", Delete, context),
            KeyBinding::new("alt-backspace", DeleteWordLeft, context),
            KeyBinding::new("alt-delete", DeleteWordRight, context),
            KeyBinding::new("cmd-backspace", DeleteToStart, context),
            KeyBinding::new("cmd-delete", DeleteToEnd, context),
            KeyBinding::new("left", Left, context),
            KeyBinding::new("right", Right, context),
            KeyBinding::new("shift-left", SelectLeft, context),
            KeyBinding::new("shift-right", SelectRight, context),
            KeyBinding::new("cmd-a", SelectAll, context),
            KeyBinding::new("cmd-v", Paste, context),
            KeyBinding::new("cmd-c", Copy, context),
            KeyBinding::new("cmd-x", Cut, context),
            KeyBinding::new("home", Home, context),
            KeyBinding::new("cmd-left", Home, context),
            KeyBinding::new("end", End, context),
            KeyBinding::new("cmd-right", End, context),
        ]);
    }
    // ⌘Z stays the overlay's (it removes the last pin) in a card's field.
    let context = Some(AREA_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("cmd-up", Top, context),
        KeyBinding::new("cmd-down", Bottom, context),
        KeyBinding::new("cmd-z", Undo, context),
        KeyBinding::new("cmd-shift-z", Redo, context),
    ]);
}

/// A field's colors.
#[derive(Clone, Copy)]
pub struct Look {
    pub text: Hsla,
    pub placeholder: Hsla,
    pub border: Hsla,
    pub background: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
}

impl Default for Look {
    /// On a card's glass: white, with only a hairline.
    fn default() -> Look {
        Look {
            text: gpui::white(),
            placeholder: hsla(0., 0., 1., 0.4),
            border: rgba(0xffffff1f).into(),
            background: hsla(0., 0., 0., 0.),
            caret: rgba(0xc2d6f6ff).into(),
            selection: rgba(0xc2d6f659).into(),
        }
    }
}

pub struct TextInput {
    focus_handle: FocusHandle,
    pub look: Look,
    pub content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<Layout>,
    last_bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    /// How far the text is scrolled up, past `max_lines`.
    scroll: Pixels,
    /// The caret and length the scroll last followed: it follows again when they change.
    followed: Option<(usize, usize)>,
    is_selecting: bool,
    /// An area: lines (Return breaks them), its own font, as tall as it's given (it scrolls past
    /// that).
    multiline: bool,
    pub max_lines: usize,
    font: Option<SharedString>,
    /// What it was before each change (text and selection), to undo, and what was undone, to redo;
    /// and when it last changed (typing goes back in one step).
    undo: Vec<(SharedString, Range<usize>)>,
    redo: Vec<(SharedString, Range<usize>)>,
    changed_at: Option<Instant>,
}

/// The text as drawn: each line (between line breaks) wrapped, where it starts in the text, and
/// how far down it's drawn.
struct Layout {
    lines: Vec<(usize, WrappedLine, Pixels)>,
    line_height: Pixels,
}

impl Layout {
    /// The rows drawn, with the wrapped lines'.
    fn rows(&self) -> usize {
        self.lines.iter().map(|(_, l, _)| l.wrap_boundaries().len() + 1).sum::<usize>().max(1)
    }

    fn line_of(&self, index: usize) -> Option<&(usize, WrappedLine, Pixels)> {
        self.lines.iter().rev().find(|(start, _, _)| *start <= index)
    }

    /// Where `index` is drawn, from the text's top left.
    fn position(&self, index: usize) -> Option<Point<Pixels>> {
        let (start, line, top) = self.line_of(index)?;
        let at = line.position_for_index(index - start, self.line_height)?;
        Some(point(at.x, at.y + *top))
    }

    /// The line drawn at `at` (from the text's top left), the point in it, and where it starts.
    fn line_at(&self, at: Point<Pixels>) -> Option<(usize, &WrappedLine, Point<Pixels>)> {
        let (start, line, top) = self.lines.iter().rev().find(|(_, _, top)| *top <= at.y).or(self.lines.first())?;
        let height = self.line_height * (line.wrap_boundaries().len() + 1) as f32;
        let y = (at.y - *top).max(px(0.)).min(height - px(1.));
        Some((*start, line, point(at.x, y)))
    }

    /// The text's index nearest `at`.
    fn closest(&self, at: Point<Pixels>) -> usize {
        let Some((start, line, local)) = self.line_at(at) else { return 0 };
        let (Ok(index) | Err(index)) = line.closest_index_for_position(local, self.line_height);
        start + index
    }

    /// The index of the character at `at`, when there's one.
    fn index_at(&self, at: Point<Pixels>) -> Option<usize> {
        let (start, line, local) = self.line_at(at)?;
        line.index_for_position(local, self.line_height).ok().map(|i| start + i)
    }
}

impl TextInput {
    pub fn new(content: &str, placeholder: &str, cx: &mut Context<Self>) -> TextInput {
        TextInput {
            focus_handle: cx.focus_handle(),
            look: Look::default(),
            content: content.to_string().into(),
            placeholder: placeholder.to_string().into(),
            selected_range: content.len()..content.len(),
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            line_height: px(18.),
            scroll: px(0.),
            followed: None,
            is_selecting: false,
            multiline: false,
            max_lines: MAX_LINES,
            font: None,
            undo: Vec::new(),
            redo: Vec::new(),
            changed_at: None,
        }
    }

    /// An area of lines in `font`, the caret at the start, as tall as its place (`flex_1` in a
    /// column): what doesn't fit scrolls.
    pub fn area(content: &str, font: &str, cx: &mut Context<Self>) -> TextInput {
        TextInput {
            multiline: true,
            font: Some(font.to_string().into()),
            selected_range: 0..0,
            ..TextInput::new(content, "", cx)
        }
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }

    /// Replaces the text, the caret at its end (an area's at its start), with nothing to undo.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string().into();
        let caret = if self.multiline { 0 } else { text.len() };
        self.selected_range = caret..caret;
        self.marked_range = None;
        (self.undo, self.redo, self.changed_at) = (Vec::new(), Vec::new(), None);
        cx.notify();
    }

    /// Types `text` where the caret is (over the selection).
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.selected_range.clone();
        self.remember(true);
        self.content = (self.content[..range.start].to_owned() + text + &self.content[range.end..]).into();
        self.selected_range = range.start + text.len()..range.start + text.len();
        self.marked_range = None;
        cx.notify();
    }

    /// Keeps what it is now, to undo the change about to be made (typing, within `UNDO_GROUP` of
    /// the last change, goes back with it).
    fn remember(&mut self, typing: bool) {
        if !self.multiline {
            return;
        }
        let grouped = typing && self.changed_at.is_some_and(|at| at.elapsed() < UNDO_GROUP);
        if !grouped {
            self.undo.push((self.content.clone(), self.selected_range.clone()));
        }
        self.redo.clear();
        self.changed_at = typing.then(Instant::now);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, selection)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.content, content), self.selected_range.clone()));
            (self.selected_range, self.marked_range, self.changed_at) = (selection, None, None);
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, selection)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.content, content), self.selected_range.clone()));
            (self.selected_range, self.marked_range, self.changed_at) = (selection, None, None);
            cx.notify();
        }
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    /// The caret a row up or down (`rows`), where it is across.
    fn vertical(&self, rows: f32) -> Option<usize> {
        let layout = self.last_layout.as_ref()?;
        let at = layout.position(self.cursor_offset())?;
        let y = at.y + layout.line_height * (rows + 0.5);
        Some(match y {
            y if y < px(0.) => 0,
            y if y >= layout.line_height * layout.rows() as f32 => self.content.len(),
            y => layout.closest(point(at.x, y)),
        })
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(to) = self.vertical(-1.) {
            self.move_to(to, cx);
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(to) = self.vertical(1.) {
            self.move_to(to, cx);
        }
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(to) = self.vertical(-1.) {
            self.select_to(to, cx);
        }
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(to) = self.vertical(1.) {
            self.select_to(to, cx);
        }
    }

    fn top(&mut self, _: &Top, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn bottom(&mut self, _: &Bottom, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    /// To the start of the line (in an area, the one the caret is on).
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(line_start(&self.content, self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(line_end(&self.content, self.cursor_offset()), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(word_start(&self.content, self.cursor_offset()), window, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(word_end(&self.content, self.cursor_offset()), window, cx);
    }

    fn delete_to_start(&mut self, _: &DeleteToStart, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(line_start(&self.content, self.cursor_offset()), window, cx);
    }

    fn delete_to_end(&mut self, _: &DeleteToEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(line_end(&self.content, self.cursor_offset()), window, cx);
    }

    /// Deletes from the caret to `offset`, or what's selected.
    fn delete_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(offset, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = match self.multiline {
                true => text.replace("\r\n", "\n"),
                false => text.replace('\n', " "),
            };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Kept within the text when it's drawn next.
        self.scroll = (self.scroll - event.delta.pixel_delta(self.line_height).y).max(px(0.));
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed { self.selected_range.start } else { self.selected_range.end }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(layout)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else {
            return 0;
        };
        let local = position - bounds.origin + point(px(0.), self.scroll);
        if local.y < px(0.) {
            return 0;
        }
        if local.y >= self.line_height * layout.rows() as f32 {
            return self.content.len();
        }
        layout.closest(local)
    }

    /// Where `index` is drawn, from the top-left corner of the field (scrolled).
    fn position(&self, index: usize) -> Point<Pixels> {
        let at = self.last_layout.as_ref().and_then(|l| l.position(index));
        at.unwrap_or_default() - point(px(0.), self.scroll)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(idx, _)| (idx < offset).then_some(idx)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected_range), reversed: self.selection_reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // One line: Return saves the card. An area takes lines.
        let new_text = match self.multiline {
            true => new_text.replace('\r', ""),
            false => new_text.replace(['\n', '\r'], ""),
        };
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        // A character typed or deleted goes back with the ones around it; more, on its own.
        if self.marked_range.is_none() {
            let one = |n: usize| n <= 1;
            self.remember(
                new_text != "\n" && one(new_text.chars().count()) && one(self.content[range.clone()].chars().count()),
            );
        }
        self.content = (self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..]).into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        self.marked_range = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|new| new.start + range.start..new.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let (start, end) = (self.position(range.start), self.position(range.end));
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin + point(end.x.max(start.x), end.y + self.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&point)? + gpui::point(px(0.), self.scroll);
        let utf8_index = self.last_layout.as_ref()?.index_at(local)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

/// The text's size and line height, as the window styles it here.
fn metrics(window: &Window) -> Metrics {
    Metrics { font_size: window.text_style().font_size.to_pixels(window.rem_size()), line_height: window.line_height() }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: Option<Layout>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    scroll: Pixels,
    followed: (usize, usize),
    /// Past `MAX_LINES`: where in the text the field shows.
    thumb: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl TextElement {
    /// What's shown (the text, or the placeholder) and how: one run, or three around marked text.
    fn runs(&self, window: &Window, cx: &App) -> (SharedString, Vec<TextRun>) {
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.clone(), input.look.placeholder)
        } else {
            (input.content.clone(), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked_range.as_ref() {
            Some(marked) => vec![
                TextRun { len: marked.start, ..run.clone() },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.0), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: text.len() - marked.end, ..run },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        (text, runs)
    }
}

/// The text wrapped at `width`, line by line.
fn shape(
    text: SharedString,
    runs: &[TextRun],
    metrics: Metrics,
    width: Option<Pixels>,
    window: &Window,
) -> Option<Layout> {
    let shaped = window.text_system().shape_text(text, metrics.font_size, runs, width, None).ok()?;
    let (mut start, mut top) = (0, px(0.));
    let mut lines = Vec::new();
    for line in shaped {
        let (len, rows) = (line.len(), line.wrap_boundaries().len() + 1);
        lines.push((start, line, top));
        start += len + 1;
        top += metrics.line_height * rows as f32;
    }
    Some(Layout { lines, line_height: metrics.line_height })
}

/// The field's font size and line height, taken where its style applies: the layout is measured
/// later, outside it, and must wrap the text exactly as it's drawn.
#[derive(Clone, Copy)]
struct Metrics {
    font_size: Pixels,
    line_height: Pixels,
}

impl Element for TextElement {
    type RequestLayoutState = Metrics;
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        // An area takes the height it's given.
        if self.input.read(cx).multiline {
            style.size.height = relative(1.).into();
            return (window.request_layout(style, None, cx), metrics(window));
        }
        let (text, runs) = self.runs(window, cx);
        let metrics = metrics(window);
        let max_lines = self.input.read(cx).max_lines.max(1);
        // As tall as the wrapped text, up to `max_lines`: a line more only once the text wraps.
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let lines = shape(text.clone(), &runs, metrics, width, window).map_or(1, |l| l.rows()).min(max_lines);
            size(width.unwrap_or_default(), metrics.line_height * lines as f32)
        });
        (layout, metrics)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        metrics: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let (text, runs) = self.runs(window, cx);
        let layout = shape(text, &runs, *metrics, Some(bounds.size.width), window);
        let line_height = metrics.line_height;
        let input = self.input.read(cx);
        let selected = input.selected_range.clone();
        let position = |index: usize| layout.as_ref().and_then(|l| l.position(index));
        // Past `max_lines`: scrolled to keep the caret in view as it moves or the text changes,
        // otherwise where the wheel left it.
        let total = line_height * layout.as_ref().map_or(1, Layout::rows) as f32;
        let mut scroll = input.scroll.min((total - bounds.size.height).max(px(0.)));
        let followed = (input.cursor_offset(), input.content.len());
        if input.followed != Some(followed) {
            let caret = position(input.cursor_offset()).unwrap_or_default().y;
            scroll = scroll.min(caret).max(caret + line_height - bounds.size.height);
        }
        let thumb = (total > bounds.size.height).then(|| {
            let height = bounds.size.height * (bounds.size.height / total);
            let top = bounds.top() + bounds.size.height * (scroll / total);
            fill(Bounds::new(point(bounds.right() + px(4.), top), size(px(2.), height)), input.look.placeholder)
        });
        let at = |index: usize| bounds.origin + position(index).unwrap_or_default() - point(px(0.), scroll);
        let (start, end) = (at(selected.start), at(selected.end));
        let (selection, cursor) = if selected.is_empty() {
            let caret = Bounds::new(at(input.cursor_offset()), size(px(2.), line_height));
            (Vec::new(), Some(fill(caret, input.look.caret)))
        } else {
            // The selected part of each line it spans.
            let tint = input.look.selection;
            let row = |from: Pixels, to: Pixels, y: Pixels| {
                fill(Bounds::from_corners(point(from, y), point(to, y + line_height)), tint)
            };
            let mut quads = Vec::new();
            if start.y == end.y {
                quads.push(row(start.x, end.x, start.y));
            } else {
                quads.push(row(start.x, bounds.right(), start.y));
                let mut y = start.y + line_height;
                while y < end.y {
                    quads.push(row(bounds.left(), bounds.right(), y));
                    y += line_height;
                }
                quads.push(row(bounds.left(), end.x, end.y));
            }
            (quads, None)
        };
        PrepaintState { layout, cursor, selection, scroll, followed, thumb }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        metrics: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let line_height = metrics.line_height;
        let (scroll, followed) = (prepaint.scroll, prepaint.followed);
        let layout = prepaint.layout.take();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            for (_, line, top) in layout.iter().flat_map(|l| &l.lines) {
                // Only the lines that show.
                let origin = bounds.origin + point(px(0.), *top - scroll);
                let height = line_height * (line.wrap_boundaries().len() + 1) as f32;
                if origin.y + height >= bounds.top() && origin.y <= bounds.bottom() {
                    let _ = line.paint(origin, line_height, TextAlign::Left, Some(bounds), window, cx);
                }
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        if let Some(thumb) = prepaint.thumb.take() {
            window.paint_quad(thumb);
        }
        self.input.update(cx, |input, _| {
            input.last_layout = layout;
            input.last_bounds = Some(bounds);
            input.line_height = line_height;
            input.scroll = scroll;
            input.followed = Some(followed);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let area = self.multiline;
        div()
            .flex()
            .key_context(if area { AREA_CONTEXT } else { CONTEXT })
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_to_start))
            .on_action(cx.listener(Self::delete_to_end))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .when(area, |d| {
                d.on_action(cx.listener(Self::newline))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::select_up))
                    .on_action(cx.listener(Self::select_down))
                    .on_action(cx.listener(Self::top))
                    .on_action(cx.listener(Self::bottom))
                    .on_action(cx.listener(Self::undo))
                    .on_action(cx.listener(Self::redo))
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .w_full()
            .px_2()
            .py_1p5()
            .rounded_md()
            .bg(self.look.background)
            .border_1()
            .border_color(self.look.border)
            .text_size(px(13.))
            .line_height(px(if area { 20. } else { 18. }))
            .when(area, |d| d.flex_1().min_h_0().px(px(12.)).py(px(10.)).rounded(px(8.)))
            .when_some(self.font.clone(), |d, font| d.font_family(font))
            .text_color(self.look.text)
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Where the line `at` is on starts in `text` (a field's one paragraph, or an area's line).
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// Where the line `at` is on ends.
fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// The start of the word before `at`, past the spaces and punctuation between, as ⌥⌫ goes.
fn word_start(text: &str, at: usize) -> usize {
    text[..at]
        .split_word_bound_indices()
        .rev()
        .find(|(_, w)| w.chars().any(char::is_alphanumeric))
        .map_or(0, |(i, _)| i)
}

/// The end of the word after `at`, past the spaces and punctuation between.
fn word_end(text: &str, at: usize) -> usize {
    text[at..]
        .split_word_bound_indices()
        .find(|(_, w)| w.chars().any(char::is_alphanumeric))
        .map_or(text.len(), |(i, w)| at + i + w.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_to_the_line_and_word_ends() {
        let text = "Keep the label, on one line\nand this";
        let caret = text.find(" line").unwrap();
        assert_eq!(line_start(text, caret), 0);
        assert_eq!(line_end(text, caret), text.find('\n').unwrap());
        assert_eq!(line_start(text, text.len()), text.find('\n').unwrap() + 1);
        // A word back from after "label, ": "label" (the comma and space go with it).
        let after_comma = text.find(" on").unwrap() + 1;
        assert_eq!(word_start(text, after_comma), text.find("label").unwrap());
        assert_eq!(word_end(text, text.find(',').unwrap()), text.find(" one").unwrap());
        // The ends stop it; accented and joined letters are one word.
        assert_eq!((word_start(text, 0), word_end(text, text.len())), (0, text.len()));
        assert_eq!(word_start("să fie", 3), 0);
        assert_eq!(word_start("don't", 5), 0);
    }
}
