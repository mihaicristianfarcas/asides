//! A clip: what happened on the live screen, as a few frames an agent can read. A frozen screen
//! can't show a flash, an animation that stutters or things happening in the wrong order; a video
//! can, but agents don't watch videos. So while the user records (the pill's button, live), the
//! app hands every frame the app in front shows to a [`Recorder`], which keeps the few that tell
//! what happened, and [`strip`] lays those out in one picture, each with its number and its time.
//!
//! **Which frames.** A recorder never holds more than [`KEPT`] frames and the newest, whatever the
//! clip's length, and of each only what it changed of the first (most of a screen stays as it was):
//! three whole frames at most, the first and the last two, to tell what the next one changes.
//! It keeps frames evenly apart in how much changed between them (the pixels that differ from one
//! frame the screen showed to the next, added up): of a slow animation, frames spread along it;
//! of a wait and then a burst, the burst. Full, it keeps every other one. A frame the screen showed
//! once and took back (the frame after it is like the one before it: a flash, a flicker) always
//! stays, when it's large enough to be seen (not a text caret blinking): it's what the eye misses.
//! So do the first and the last: how it was before, and how it ended. A frame the same as the one
//! before isn't kept at all.
//!
//! **What of them.** Only the part of the screen that changed at some point, with a little around
//! it: the frames are as large as they can be in the strip.

use crate::measure::{Frame, Rect};
use crate::picture;

/// Frames a recorder holds while recording.
pub const KEPT: usize = 16;
/// Frames in a clip's strip.
pub const MAX_FRAMES: usize = 9;
/// A pixel changed when its channels differ by more than this in all (the screen's own dithering
/// and a scaled capture's rounding don't count).
const NOISE: u32 = 12;
/// How much more alike the frames around a frame must be than it is to either, for it to be one the
/// screen showed and took back; and how many pixels it must have changed (a caret blinks 40).
const TAKEN_BACK: u64 = 4;
const FLASH_MIN: u64 = 200;
/// Pixels kept around what changed, and the least a frame of the strip shows, in the frames' pixels.
const AROUND: u32 = 24;
const MIN_SIDE: u32 = 200;
/// The strip: its longer side at most (as an aside's screenshots), the space between and around its
/// frames, the band over each frame for its number and time, and its colors.
pub const STRIP_SIDE: u32 = 1568;
const GAP: u32 = 10;
const BAND: u32 = 24;
const BACKGROUND: u32 = 0x1c1c1e;
const INK: u32 = 0xf2f2f7;

/// The longer side of a frame handed to a recorder, at most: a screen with more pixels is recorded
/// smaller (a strip's frames are smaller still, unless little of the screen changed).
pub const SIDE: u32 = 2000;

/// A frame kept, when it showed (seconds into the clip), how much had changed on the screen by
/// then (pixels, added up from frame to frame), and whether the screen took it back at once.
struct Kept {
    time: f32,
    at: u64,
    flash: bool,
    /// What it showed where the screen had changed at all by then (`Recorder::touched`); the rest is
    /// as the first frame showed it. `None`: the first frame itself.
    patch: Option<(Rect, Frame)>,
}

/// Keeps what tells what happened of the frames it's given (see the module's docs).
pub struct Recorder {
    /// The first frame, whole.
    first: Option<Frame>,
    /// The frame the screen shows now, when it isn't the first: the next one is compared with it.
    shown: Option<Frame>,
    /// The frames kept, oldest first: at least `step` apart in how much changed, flashes besides.
    kept: Vec<Kept>,
    /// The newest frame, when it isn't one of them (it's the clip's last, if no other comes).
    newest: Option<Kept>,
    step: u64,
    /// How much had changed by the last frame kept for being `step` from the one before.
    anchor: u64,
    /// How much has changed so far.
    changed_by: u64,
    /// The part of the screen that changed, over all the frames seen: where a pixel changed enough
    /// to be seen (what a clip shows), and where one changed at all (what a kept frame holds: an
    /// edge's soft pixels, left out, stayed behind as a ghost of where a thing first was).
    changed: Option<Rect>,
    touched: Option<Rect>,
    /// Parts of the screen that aren't the user's (asides' own pill, where a capture can't leave it
    /// out): they stay as the first frame showed them.
    ignored: Vec<Rect>,
    /// How many different frames there were, and when the last one showed.
    seen: usize,
    last: f32,
    /// The frame the screen showed before that one, and how many pixels that one changed of it: the
    /// next frame says whether it was taken back.
    previous: Option<(Frame, u64)>,
}

/// What a recorder kept: the frames that tell what happened, each with when it showed (seconds
/// into the clip), all cut to the part of the screen that changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub frames: Vec<(f32, Frame)>,
    /// The part of the screen they show, in the recorded frames' pixels.
    pub rect: Rect,
    /// How many different frames the screen showed, and for how long it was recorded.
    pub seen: usize,
    pub seconds: f32,
}

impl Default for Recorder {
    fn default() -> Recorder {
        Recorder::ignoring(Vec::new())
    }
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// A recorder that takes no notice of what happens in `ignored` (frames' pixels).
    pub fn ignoring(ignored: Vec<Rect>) -> Recorder {
        Recorder {
            first: None,
            shown: None,
            kept: Vec::new(),
            newest: None,
            step: 1,
            anchor: 0,
            changed_by: 0,
            changed: None,
            touched: None,
            ignored,
            seen: 0,
            last: 0.0,
            previous: None,
        }
    }

    /// The screen at `time` (seconds into the clip). Frames of another size than the first aren't
    /// taken (the display changed).
    pub fn push(&mut self, time: f32, mut frame: Frame) {
        self.last = self.last.max(time);
        let Some(first) = &self.first else {
            self.seen = 1;
            self.kept.push(Kept { time, at: 0, flash: false, patch: None });
            self.first = Some(frame);
            return;
        };
        if (first.width, first.height) != (frame.width, frame.height) {
            return;
        }
        // What isn't the user's stays as it first was.
        for rect in &self.ignored {
            copy(first, &mut frame, *rect);
        }
        let before = self.shown.as_ref().unwrap_or(first);
        let Some(Difference { rect, pixels: change, touched }) = difference(before, &frame) else { return };
        // Back to how it was before the newest frame: that one was a flash, and stays.
        let taken_back = self.previous.as_ref().is_some_and(|(earlier, changed)| {
            let back = difference(earlier, &frame).map_or(0, |d| d.pixels);
            change.min(*changed) >= FLASH_MIN && back * TAKEN_BACK < change.min(*changed)
        });
        match self.newest.take() {
            Some(flash) if taken_back => self.kept.push(Kept { flash: true, ..flash }),
            None if taken_back && self.kept.len() > 1 => self.kept.last_mut().expect("not empty").flash = true,
            _ => {}
        }
        self.seen += 1;
        self.changed_by += change;
        self.changed = Some(self.changed.map_or(rect, |r| union(r, rect)));
        let touched = self.touched.map_or(touched, |r| union(r, touched));
        self.touched = Some(touched);
        // Outside what changed so far, it's the first frame.
        let patch = picture::crop(&frame, touched).map(|pixels| (touched, pixels));
        let before = self.shown.replace(frame).unwrap_or_else(|| first.clone());
        self.previous = Some((before, change));
        let frame = Kept { time, at: self.changed_by, flash: false, patch };
        if self.changed_by - self.anchor >= self.step {
            self.anchor = self.changed_by;
            self.kept.push(frame);
        } else {
            self.newest = Some(frame);
        }
        // Full: every other one, until they fit.
        while self.kept.len() > KEPT {
            self.step *= 2;
            self.anchor = space(&mut self.kept, self.step);
            if self.step > self.changed_by {
                thin_flashes(&mut self.kept, KEPT);
            }
        }
    }

    /// How many frames it holds.
    pub fn len(&self) -> usize {
        self.kept.len() + self.newest.iter().len()
    }

    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    /// The clip `seconds` long: at most [`MAX_FRAMES`] frames, cut to what changed. `None` when no
    /// frame came.
    pub fn finish(mut self, seconds: f32) -> Option<Clip> {
        self.kept.extend(self.newest.take());
        let first = self.first.take()?;
        let kept = choose(self.kept, MAX_FRAMES);
        let whole = Rect::new(0, 0, first.width, first.height);
        // Nothing changed: the screen, as it stayed.
        let rect = self.changed.map_or(whole, |r| around(r, first.width, first.height));
        let frames = kept
            .iter()
            .filter_map(|k| {
                let mut frame = picture::crop(&first, rect)?;
                // The part of what it changed that the clip shows.
                if let Some((at, pixels)) = &k.patch
                    && let Some(shown) = intersection(*at, rect)
                    && let Some(part) =
                        picture::crop(pixels, Rect::new(shown.x - at.x, shown.y - at.y, shown.width, shown.height))
                {
                    paste(&mut frame, &part, shown.x - rect.x, shown.y - rect.y);
                }
                Some((k.time, frame))
            })
            .collect();
        Some(Clip { frames, rect, seen: self.seen, seconds: seconds.max(self.last) })
    }
}

/// Leaves in `kept` the first frame, the flashes, and the others at least `step` apart in how much
/// changed; says where the last of those is.
fn space(kept: &mut Vec<Kept>, step: u64) -> u64 {
    let mut anchor = 0;
    let mut first = true;
    kept.retain(|k| {
        let stays = std::mem::take(&mut first) || k.at - anchor >= step;
        if stays {
            anchor = k.at;
        }
        stays || k.flash
    });
    anchor
}

/// Takes flashes out of `kept` until `most` frames are left: each time the one nearest its
/// neighbours in time (of a hundred flickers, some from all along).
fn thin_flashes(kept: &mut Vec<Kept>, most: usize) {
    while kept.len() > most.max(2) {
        let between = |i: usize| kept[i + 1].time - kept[i - 1].time;
        let flashes = (1..kept.len() - 1).filter(|&i| kept[i].flash);
        let Some(nearest) = flashes.min_by(|&a, &b| between(a).total_cmp(&between(b))) else { return };
        kept.remove(nearest);
    }
}

/// The `most` frames of `kept` that tell what happened: the first, the last, the flashes, and of
/// the others those nearest to evenly apart in how much changed.
fn choose(mut kept: Vec<Kept>, most: usize) -> Vec<Kept> {
    if kept.len() <= most {
        return kept;
    }
    let last = kept.len() - 1;
    let flashes = (1..last).filter(|&i| kept[i].flash).count();
    // More flashes than fit: some from all along, and nothing else between the ends.
    if flashes + 2 >= most {
        kept.retain({
            let mut i = 0;
            move |k| {
                i += 1;
                i == 1 || i == last + 1 || k.flash
            }
        });
        thin_flashes(&mut kept, most);
        return kept;
    }
    let room = most - 2 - flashes;
    let total = kept[last].at;
    let mut chosen: Vec<usize> = Vec::new();
    for slot in 1..=room {
        let wanted = total * slot as u64 / (room as u64 + 1);
        let free = (1..last).filter(|i| !kept[*i].flash && !chosen.contains(i));
        chosen.extend(free.min_by_key(|&i| kept[i].at.abs_diff(wanted)));
    }
    let mut i = 0;
    kept.retain(|k| {
        i += 1;
        i == 1 || i == last + 1 || k.flash || chosen.contains(&(i - 1))
    });
    kept
}

/// Where two frames differ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Difference {
    /// The smallest rectangle around the pixels that changed enough to be seen, and how many.
    pub rect: Rect,
    pub pixels: u64,
    /// The smallest rectangle around every pixel that isn't the same.
    pub touched: Rect,
}

/// Where `a` and `b` (the same size) differ. `None` when they show the same: no pixel changed
/// enough to be seen.
pub fn difference(a: &Frame, b: &Frame) -> Option<Difference> {
    let row = a.width as usize * 4;
    // Left, right, top, bottom: of what's seen, and of what isn't the same.
    let (mut seen, mut any) = ([u32::MAX, 0, u32::MAX, 0], [u32::MAX, 0, u32::MAX, 0]);
    let grow = |around: &mut [u32; 4], x: u32, y: u32| {
        *around = [around[0].min(x), around[1].max(x), around[2].min(y), around[3].max(y)];
    };
    let mut pixels = 0u64;
    for (y, (ra, rb)) in a.rgba.chunks_exact(row.max(1)).zip(b.rgba.chunks_exact(row.max(1))).enumerate() {
        // Most rows are the same from one frame to the next.
        if ra == rb {
            continue;
        }
        for (x, (pa, pb)) in ra.as_chunks::<4>().0.iter().zip(rb.as_chunks::<4>().0).enumerate() {
            if pa == pb {
                continue;
            }
            grow(&mut any, x as u32, y as u32);
            let apart = |c: usize| (pa[c] as i32 - pb[c] as i32).unsigned_abs();
            if apart(0) + apart(1) + apart(2) > NOISE {
                grow(&mut seen, x as u32, y as u32);
                pixels += 1;
            }
        }
    }
    let rect = |[x0, x1, y0, y1]: [u32; 4]| Rect::from_corners((x0, y0), (x1, y1));
    (pixels > 0).then(|| Difference { rect: rect(seen), pixels, touched: rect(any) })
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
}

/// The part of `a` in `b`, if any.
fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (right, bottom) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

/// `rect` with [`AROUND`] pixels around it, at least [`MIN_SIDE`] a side, inside `width` × `height`.
fn around(rect: Rect, width: u32, height: u32) -> Rect {
    let grow = |start: u32, length: u32, limit: u32| {
        let wanted = (length + 2 * AROUND).max(MIN_SIDE).min(limit);
        let middle = start + length / 2;
        let from = middle.saturating_sub(wanted / 2).min(limit - wanted);
        (from, wanted)
    };
    let ((x, w), (y, h)) = (grow(rect.x, rect.width, width), grow(rect.y, rect.height, height));
    Rect::new(x, y, w, h)
}

/// Copies `rect` of `from` into `to` (the same size).
fn copy(from: &Frame, to: &mut Frame, rect: Rect) {
    let rect = rect.clamp_to(from.width, from.height);
    let row = from.width as usize * 4;
    for y in rect.y..rect.bottom() {
        let (start, end) = (y as usize * row + rect.x as usize * 4, y as usize * row + rect.right() as usize * 4);
        to.rgba[start..end].copy_from_slice(&from.rgba[start..end]);
    }
}

/// A clip's frames in one picture, left to right then top to bottom, each under its number and its
/// time (`2  0.35s`), as large as [`STRIP_SIDE`] lets them be (never larger than recorded).
pub fn strip(clip: &Clip) -> Option<Frame> {
    let (w, h) = clip.frames.first().map(|(_, f)| (f.width, f.height))?;
    let n = clip.frames.len() as u32;
    // The layout whose frames come out largest; of those, the most compact picture.
    let layout = |columns: u32| {
        let rows = n.div_ceil(columns);
        let across = (STRIP_SIDE.saturating_sub(GAP * (columns + 1))) as f64 / (columns * w) as f64;
        let down = (STRIP_SIDE.saturating_sub(GAP * (rows + 1) + BAND * rows)) as f64 / (rows * h) as f64;
        let scale = across.min(down).min(1.0);
        let (tw, th) = (((w as f64 * scale) as u32).max(1), ((h as f64 * scale) as u32).max(1));
        (rows, tw, th, GAP + columns * (tw + GAP), GAP + rows * (BAND + th + GAP))
    };
    let columns = (1..=n).max_by(|&a, &b| {
        let ((_, wa, _, sheet_wa, sheet_ha), (_, wb, _, sheet_wb, sheet_hb)) = (layout(a), layout(b));
        wa.cmp(&wb).then(sheet_wb.max(sheet_hb).cmp(&sheet_wa.max(sheet_ha)))
    })?;
    let (_, tw, th, width, height) = layout(columns);
    let mut sheet = Frame::filled(width, height, BACKGROUND);
    for (i, (time, frame)) in clip.frames.iter().enumerate() {
        let (column, row) = (i as u32 % columns, i as u32 / columns);
        let (x, y) = (GAP + column * (tw + GAP), GAP + row * (BAND + th + GAP));
        write(&mut sheet, x + 2, y + (BAND - GLYPH.1 * 2) / 2, &format!("{}  {time:.2}s", i + 1), 2);
        let tile = if (tw, th) == (frame.width, frame.height) { frame.clone() } else { picture::scaled(frame, tw, th) };
        paste(&mut sheet, &tile, x, y + BAND);
    }
    Some(sheet)
}

/// What the strip shows, said after it in the aside: how many frames, of which app (`None`: the
/// screen), over how long, when each showed, and how to read it.
pub fn caption(clip: &Clip, app: Option<&str>) -> String {
    let times: Vec<String> = clip.frames.iter().map(|(t, _)| format!("{t:.2}")).collect();
    let place = app.map_or("on the screen".to_string(), |app| format!("in {app}"));
    match clip.frames.len() {
        0 | 1 => format!("**Clip:** nothing {place} changed in {:.1} s.", clip.seconds),
        n => {
            let of = if clip.seen > n { format!(" of the {} it showed", clip.seen) } else { String::new() };
            format!(
                "**Clip:** {n} frames{of}, what changed {place} over {:.1} s, left to right then top to bottom, at \
                 {} s.",
                clip.seconds,
                times.join(", ")
            )
        }
    }
}

fn paste(onto: &mut Frame, frame: &Frame, x: u32, y: u32) {
    let (row, width) = (onto.width as usize * 4, frame.width.min(onto.width.saturating_sub(x)) as usize * 4);
    for (line, pixels) in frame.rgba.chunks_exact(frame.width as usize * 4).enumerate() {
        let start = (y as usize + line) * row + x as usize * 4;
        if start + width <= onto.rgba.len() {
            onto.rgba[start..start + width].copy_from_slice(&pixels[..width]);
        }
    }
}

/// A glyph's size, in its own pixels.
const GLYPH: (u32, u32) = (5, 7);

/// The digits, the point and the s of a frame's label, 5 × 7, a row a byte (the low five bits, left
/// to right): numbers an agent reads, drawn without a typeface (there's none here to draw with).
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x0c],
        's' => [0x00, 0x00, 0x0e, 0x10, 0x0e, 0x01, 0x1e],
        _ => [0; 7],
    }
}

/// Writes `text` on `onto` from `(x, y)`, each of a glyph's pixels `size` of the picture's.
fn write(onto: &mut Frame, x: u32, y: u32, text: &str, size: u32) {
    for (i, c) in text.chars().enumerate() {
        let left = x + i as u32 * (GLYPH.0 + 1) * size;
        for (row, bits) in glyph(c).iter().enumerate() {
            for column in 0..GLYPH.0 {
                if bits >> (GLYPH.0 - 1 - column) & 1 == 1 {
                    onto.fill(Rect::new(left + column * size, y + row as u32 * size, size, size), INK);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 400;
    const H: u32 = 300;

    /// A white screen with a 40 × 20 block of `color` at `x`, 100 down.
    fn screen(x: u32, color: u32) -> Frame {
        let mut frame = Frame::filled(W, H, 0xffffff);
        frame.fill(Rect::new(x, 100, 40, 20), color);
        frame
    }

    #[test]
    fn finds_where_two_frames_differ() {
        let (a, b) = (screen(100, 0x2563eb), screen(110, 0x2563eb));
        // Moved 10 right: 10 columns lost on the left, 10 gained on the right.
        let whole = Rect::new(100, 100, 50, 20);
        assert_eq!(difference(&a, &b), Some(Difference { rect: whole, pixels: 400, touched: whole }));
        assert_eq!(difference(&a, &a), None);
        // A shade no one sees isn't a change.
        assert_eq!(difference(&a, &screen(100, 0x2563ec)), None);
        // But it's part of what a frame that did change holds: a soft edge left behind showed.
        let mut soft = b.clone();
        soft.fill(Rect::new(90, 96, 1, 1), 0xfefefe);
        assert_eq!(
            difference(&a, &soft),
            Some(Difference { rect: whole, pixels: 400, touched: Rect::new(90, 96, 60, 24) })
        );
    }

    #[test]
    fn keeps_a_flash_and_how_it_was_before_and_after() {
        let mut recorder = Recorder::new();
        let still = screen(100, 0x2563eb);
        // Nothing happens for a second, the button flashes red for one frame, and nothing again.
        for i in 0..60 {
            recorder.push(i as f32 / 60.0, still.clone());
        }
        recorder.push(1.0, screen(100, 0xff0000));
        for i in 61..120 {
            recorder.push(i as f32 / 60.0, still.clone());
        }
        assert_eq!(recorder.len(), 3, "frames the same as the one before aren't kept");
        let clip = recorder.finish(2.0).unwrap();
        let times: Vec<f32> = clip.frames.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, [0.0, 1.0, 61.0 / 60.0]);
        assert_eq!((clip.seen, clip.seconds), (3, 2.0));
        // Cut to the button, with room around it: 200 at least a side.
        assert_eq!(clip.rect, Rect::new(20, 10, 200, 200));
        let (before, flash, after) = (&clip.frames[0].1, &clip.frames[1].1, &clip.frames[2].1);
        assert_eq!((flash.width, flash.height), (200, 200));
        assert_eq!((before.rgb(100, 100), flash.rgb(100, 100), after.rgb(100, 100)), (0x2563eb, 0xff0000, 0x2563eb));
        assert_eq!(
            caption(&clip, None),
            "**Clip:** 3 frames, what changed on the screen over 2.0 s, left to right then top to bottom, at 0.00, \
             1.00, 1.02 s."
        );
    }

    #[test]
    fn nothing_of_where_a_thing_was_stays_behind() {
        // A block with a soft edge (a row a shade off white, as antialiasing leaves) moves away.
        let soft = |x: u32| {
            let mut frame = screen(x, 0x2563eb);
            frame.fill(Rect::new(x, 99, 40, 1), 0xfcfcfc);
            frame
        };
        let mut recorder = Recorder::new();
        recorder.push(0.0, soft(100));
        recorder.push(0.5, soft(200));
        let clip = recorder.finish(1.0).unwrap();
        let after = &clip.frames[1].1;
        // Where it was is white again, its soft edge too (too faint to count as a change itself).
        let (x, y) = (110 - clip.rect.x, 99 - clip.rect.y);
        assert_eq!((clip.frames[0].1.rgb(x, y), after.rgb(x, y), after.rgb(x, y + 5)), (0xfcfcfc, 0xffffff, 0xffffff));
    }

    #[test]
    fn a_long_animation_stays_within_its_frames_with_the_flash_in_it() {
        let mut recorder = Recorder::new();
        // The block slides 300 pixels right in 300 frames; at frame 150 it's red for one frame.
        for i in 0..300u32 {
            let color = if i == 150 { 0xff0000 } else { 0x2563eb };
            recorder.push(i as f32 / 60.0, screen(20 + i, color));
            // The frames kept, and the newest.
            assert!(recorder.len() <= KEPT + 1);
        }
        let clip = recorder.finish(5.0).unwrap();
        assert_eq!((clip.frames.len(), clip.seen), (MAX_FRAMES, 300));
        let times: Vec<f32> = clip.frames.iter().map(|(t, _)| *t).collect();
        // How it started, how it ended, and the flash.
        assert_eq!((times[0], times[MAX_FRAMES - 1]), (0.0, 299.0 / 60.0));
        assert!(times.contains(&2.5), "the flash went: {times:?}");
        assert!(times.windows(2).all(|t| t[0] < t[1]));
        // The rest spread over the slide: no two of them further apart than a third of it.
        assert!(times.windows(2).all(|t| t[1] - t[0] < 5.0 / 3.0), "{times:?}");
        // All of the slide's path is in the frames, with room around it.
        assert!(clip.rect.x == 0 && clip.rect.right() >= 383, "{:?}", clip.rect);
        assert!(
            caption(&clip, Some("Safari"))
                .starts_with("**Clip:** 9 frames of the 300 it showed, what changed in Safari over 5.0 s")
        );
    }

    #[test]
    fn ignores_what_is_not_the_users_and_says_when_nothing_changed() {
        // asides' own pill, counting the seconds at the top: not what happened on the screen.
        let pill = Rect::new(150, 0, 100, 30);
        let mut recorder = Recorder::ignoring(vec![pill]);
        for i in 0..5 {
            let mut frame = screen(100, 0x2563eb);
            frame.fill(Rect::new(160 + i * 4, 5, 20, 20), 0x111111);
            recorder.push(i as f32, frame);
        }
        assert_eq!(recorder.len(), 1);
        let clip = recorder.finish(4.5).unwrap();
        assert_eq!((clip.frames.len(), clip.rect), (1, Rect::new(0, 0, W, H)));
        assert_eq!(caption(&clip, None), "**Clip:** nothing on the screen changed in 4.5 s.");
        assert_eq!(caption(&clip, Some("Xcode")), "**Clip:** nothing in Xcode changed in 4.5 s.");
        assert!(Recorder::new().finish(1.0).is_none());
        // A frame of another size (the display changed) isn't taken.
        let mut recorder = Recorder::new();
        recorder.push(0.0, Frame::filled(10, 10, 0));
        recorder.push(0.1, Frame::filled(20, 10, 0xffffff));
        assert_eq!(recorder.len(), 1);
    }

    /// With `ASIDES_STRIP_DUMP=<dir>`, writes `strip.png` there: a block sliding across a card and
    /// flashing once, as a clip's strip, to look at.
    #[test]
    fn dumps_a_strip() {
        let Some(dir) = std::env::var_os("ASIDES_STRIP_DUMP").map(std::path::PathBuf::from) else { return };
        let mut recorder = Recorder::new();
        for i in 0..240u32 {
            let mut frame = Frame::filled(900, 500, 0xf3f4f6);
            frame.fill(Rect::new(60, 120, 780, 220), 0xffffff);
            let color = if i == 130 { 0xdc2626 } else { 0x2563eb };
            frame.fill(Rect::new(80 + i * 2, 200, 160, 56), color);
            recorder.push(i as f32 / 60.0, frame);
        }
        let clip = recorder.finish(4.0).unwrap();
        std::fs::write(dir.join("strip.png"), picture::png(&strip(&clip).unwrap())).unwrap();
        std::fs::write(dir.join("strip.txt"), caption(&clip, None)).unwrap();
    }

    #[test]
    fn lays_the_frames_out_in_one_picture() {
        let frame = |color: u32| Frame::filled(300, 200, color);
        let clip = |n: usize| Clip {
            frames: (0..n).map(|i| (i as f32 * 0.25, frame(0x100000 * (i as u32 + 1)))).collect(),
            rect: Rect::new(0, 0, 300, 200),
            seen: n,
            seconds: 2.0,
        };
        // Three as recorded, in the most compact picture: two and one.
        let sheet = strip(&clip(3)).unwrap();
        assert_eq!((sheet.width, sheet.height), (GAP + 2 * (300 + GAP), GAP + 2 * (BAND + 200 + GAP)));
        assert_eq!(sheet.rgb(GAP, GAP + BAND), 0x100000);
        assert_eq!(sheet.rgb(GAP + 310, GAP + BAND + 199), 0x200000);
        assert_eq!(sheet.rgb(GAP, GAP + BAND + 200 + GAP + BAND), 0x300000);
        assert_eq!(sheet.rgb(0, 0), BACKGROUND);
        // Each under its number and time: the 1's stem is in its band.
        assert_eq!(sheet.rgb(GAP + 2 + 2 * 2, GAP + (BAND - 14) / 2), INK);
        // Nine go three by three, still as recorded; never past the strip's side.
        let sheet = strip(&clip(9)).unwrap();
        assert_eq!((sheet.width, sheet.height), (GAP + 3 * (300 + GAP), GAP + 3 * (BAND + 200 + GAP)));
        let wide = Clip { frames: (0..4).map(|i| (i as f32, Frame::filled(1400, 500, 0x336699))).collect(), ..clip(4) };
        let sheet = strip(&wide).unwrap();
        assert!(sheet.width <= STRIP_SIDE && sheet.height <= STRIP_SIDE, "{} × {}", sheet.width, sheet.height);
        // Smaller, they're still what they were.
        assert_eq!(sheet.rgb(GAP + 5, GAP + BAND + 5), 0x336699);
        assert!(strip(&Clip { frames: Vec::new(), ..clip(0) }).is_none());
    }
}
