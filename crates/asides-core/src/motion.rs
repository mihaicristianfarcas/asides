//! A clip: what happened on the live screen, as a few frames an agent can read. A frozen screen
//! can't show a flash, an animation that stutters or things happening in the wrong order; a video
//! can, but agents don't watch videos. So while the user records (the pill's button, live), the
//! app hands every frame the app in front shows to a [`Recorder`], which keeps the few that tell
//! what happened, and [`pictures`] lays those out, each with its number and its time.
//!
//! **Which frames.** A recorder never holds more than [`KEPT`] frames, the newest and two of the
//! movement going on, whatever the clip's length, and of each only what it changed of the first
//! (most of a screen stays as it was): a few whole frames at most, the first, the last two and the
//! last one settled on, to tell what the next one changes. These always stay:
//! - the first and the last: how it was before, and how it ended;
//! - each screen the app settled on, held for [`REST`] at least (a row selected, a menu open, a page
//!   loaded), unless it looks like the last one did (a text caret blinking);
//! - each frame the screen showed for an instant and took back (the frame after it is like the one
//!   before it: a flash, a flicker), when it's large enough to be seen: it's what the eye misses.
//!
//! Between them, a frame of each movement from one to the next (a menu opening, a list scrolling),
//! then frames evenly apart in how much changed (the pixels that differ from one frame the screen
//! showed to the next, added up): of a slow animation, frames spread along it. Full, it keeps every
//! other one of those; with too many settled screens and flashes, some from all along. A frame the
//! same as the one before isn't kept at all.
//!
//! **What of them.** Only the part of the screen that changed at some point, with a little around
//! it, and never much smaller than recorded ([`MIN_SCALE`]), so its text can be read: as many frames
//! as fit in one picture at that size, and more pictures for more ([`MAX_PICTURES`] at most).

use crate::measure::{Frame, Rect};
use crate::picture;

/// Frames a recorder holds while recording.
pub const KEPT: usize = 16;
/// Frames in a clip, at most, and the pictures they're laid out in.
pub const MAX_FRAMES: usize = 12;
pub const MAX_PICTURES: usize = 8;
/// A pixel changed when its channels differ by more than this in all (the screen's own dithering
/// and a scaled capture's rounding don't count).
const NOISE: u32 = 12;
/// How much more alike the frames around a frame must be than it is to either, for it to be one the
/// screen showed and took back; and how many pixels it must have changed (a caret blinks 40).
const TAKEN_BACK: u64 = 4;
const FLASH_MIN: u64 = 200;
/// How long the screen must show a frame for it to be one the app settled on, and how many pixels it
/// must differ from the last one by (a caret blinking changes 40 to 80).
pub const REST: f32 = 0.3;
const REST_MIN: u64 = 100;
/// Pixels kept around what changed, and the least a clip's frame shows, in the frames' pixels.
const AROUND: u32 = 24;
const MIN_SIDE: u32 = 200;
/// How much smaller than recorded a frame is shown, at most, unless one alone doesn't fit a picture.
pub const MIN_SCALE: f64 = 0.8;
/// A clip's picture: its longer side at most (as an aside's screenshots), the space between and
/// around its frames, the band over each frame for its number and time, and its colors.
pub const PICTURE_SIDE: u32 = 1568;
const GAP: u32 = 10;
const BAND: u32 = 24;
const BACKGROUND: u32 = 0x1c1c1e;
const INK: u32 = 0xf2f2f7;

/// The longer side of a frame handed to a recorder, at most: a screen with more pixels is recorded
/// smaller.
pub const SIDE: u32 = 2000;

/// A frame kept, when it showed (seconds into the clip), how much had changed on the screen by
/// then (pixels, added up from frame to frame), whether the screen took it back at once, whether it
/// held it (one the app settled on), and which movement it's of (counted from each time the screen
/// held a frame, or flashed).
struct Kept {
    time: f32,
    at: u64,
    flash: bool,
    rest: bool,
    movement: u32,
    /// The screen back as it was before a flash: no frame of a movement of its own.
    back: bool,
    /// What it showed where the screen had changed at all by then (`Recorder::touched`); the rest is
    /// as the first frame showed it. `None`: the first frame itself.
    patch: Option<(Rect, Frame)>,
}

/// Keeps what tells what happened of the frames it's given (see the module's docs).
pub struct Recorder {
    /// The first frame, whole.
    first: Option<Frame>,
    /// The frame the screen shows now, when it isn't the first: the next one is compared with it.
    /// And since when.
    shown: Option<Frame>,
    shown_at: f32,
    /// The last frame the app settled on, when it isn't the first: the next must differ from it.
    rest: Option<Frame>,
    /// The frames kept, oldest first: at least `step` apart in how much changed, the frames settled
    /// on and the flashes besides.
    kept: Vec<Kept>,
    /// The newest frame, when it isn't one of them (it's the clip's last, if no other comes).
    newest: Option<Kept>,
    /// The movement going on (since the screen last held a frame, or flashed), how much had changed
    /// when it started, and two of its frames not kept, taken as they went by when it had changed
    /// twice as much as by the one before: whenever it ends, one is a third to two thirds through
    /// it. Each movement keeps a frame so, however little it changed.
    movement: u32,
    moved_from: u64,
    middles: [Option<Kept>; 2],
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
    /// Which of `frames` the screen showed for an instant only (their indexes).
    pub flashes: Vec<usize>,
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
            shown_at: 0.0,
            rest: None,
            kept: Vec::new(),
            newest: None,
            movement: 0,
            moved_from: 0,
            middles: [None, None],
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
            self.shown_at = time;
            self.kept.push(Kept { time, at: 0, flash: false, rest: false, movement: 0, back: false, patch: None });
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
        // The frame shown until now held: one the app settled on. Or it didn't, and the screen is
        // back to how it was before it: a flash. Either stays.
        let held = time - self.shown_at >= REST;
        if held {
            self.settled();
        }
        let taken_back = !held
            && self.previous.as_ref().is_some_and(|(earlier, changed)| {
                let back = difference(earlier, &frame).map_or(0, |d| d.pixels);
                change.min(*changed) >= FLASH_MIN && back * TAKEN_BACK < change.min(*changed)
            });
        let since = self.shown_at;
        match self.newest.take() {
            Some(flash) if taken_back => {
                self.moved(flash.at, since);
                self.kept.push(Kept { flash: true, ..flash });
            }
            // Kept already, for how much it changed.
            None if taken_back => {
                if let Some(kept) = self.kept.last_mut().filter(|k| k.time == since) {
                    kept.flash = true;
                    let at = kept.at;
                    self.moved(at, since);
                }
            }
            // Gone by: maybe the frame of this movement (not the one it started from, held).
            Some(passed) if passed.at > self.moved_from => {
                let from = self.moved_from;
                if self.middles[1].as_ref().is_none_or(|m| passed.at - from >= 2 * (m.at - from)) {
                    self.middles = [self.middles[1].take(), Some(passed)];
                }
            }
            _ => {}
        }
        self.seen += 1;
        self.changed_by += change;
        self.changed = Some(self.changed.map_or(rect, |r| union(r, rect)));
        let touched = self.touched.map_or(touched, |r| union(r, touched));
        self.touched = Some(touched);
        // Outside what changed so far, it's the first frame.
        let patch = picture::crop(&frame, touched).map(|pixels| (touched, pixels));
        let before = match self.shown.replace(frame) {
            Some(before) => before,
            None => self.first.clone().expect("a first frame"),
        };
        self.previous = Some((before, change));
        self.shown_at = time;
        let (movement, back) = (self.movement, taken_back);
        let frame = Kept { time, at: self.changed_by, flash: false, rest: false, movement, back, patch };
        if self.changed_by - self.anchor >= self.step {
            self.anchor = self.changed_by;
            self.kept.push(frame);
        } else {
            self.newest = Some(frame);
        }
        // Full: every other one, until they fit.
        while self.kept.len() > KEPT {
            self.step = self.step.saturating_mul(2);
            self.anchor = space(&mut self.kept, self.step);
            if self.step > self.changed_by {
                thin(&mut self.kept, KEPT);
            }
        }
    }

    /// The frame shown until now held ([`REST`]): a movement ended. The frame is kept as one the
    /// app settled on, unless it differs little from the last one (a caret blinked, or it's back to
    /// how it was), or it's the first (kept anyway).
    fn settled(&mut self) {
        let since = self.shown_at;
        self.moved(self.changed_by, since);
        let (Some(first), Some(shown)) = (&self.first, &self.shown) else { return };
        let last = self.rest.as_ref().unwrap_or(first);
        if difference(last, shown).map_or(0, |d| d.pixels) < REST_MIN {
            return;
        }
        self.rest = Some(shown.clone());
        match self.newest.take() {
            Some(newest) => self.kept.push(Kept { rest: true, ..newest }),
            // Kept already, for how much it changed.
            None => {
                if let Some(kept) = self.kept.last_mut().filter(|k| k.time == since) {
                    kept.rest = true;
                }
            }
        }
    }

    /// A movement ended at `time` (how much had changed by then: `at`), and another starts: the
    /// frame kept of it (the one nearer halfway through) goes among the others, unless it changed too
    /// little to see (a caret blinked).
    fn moved(&mut self, at: u64, time: f32) {
        let (from, seen) = (self.moved_from, at.saturating_sub(self.moved_from) >= REST_MIN);
        let half = from.midpoint(at);
        let [earlier, later] = std::mem::take(&mut self.middles);
        let middle = [earlier, later].into_iter().flatten().min_by_key(|m| m.at.abs_diff(half));
        if let Some(middle) = middle.filter(|m| seen && m.time < time) {
            let place = self.kept.partition_point(|k| k.time < middle.time);
            self.kept.insert(place, middle);
        }
        self.moved_from = at;
        self.movement += 1;
    }

    /// How many frames it holds.
    pub fn len(&self) -> usize {
        self.kept.len() + self.newest.iter().len() + self.middles.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    /// The clip `seconds` long: cut to what changed, at most [`MAX_FRAMES`] frames, and no more
    /// than [`MAX_PICTURES`] hold. `None` when no frame came.
    pub fn finish(mut self, seconds: f32) -> Option<Clip> {
        self.kept.extend(self.newest.take());
        self.moved(self.changed_by, f32::INFINITY);
        let first = self.first.take()?;
        let whole = Rect::new(0, 0, first.width, first.height);
        // Nothing changed: the screen, as it stayed.
        let rect = self.changed.map_or(whole, |r| around(r, first.width, first.height));
        let most = MAX_FRAMES.min(per_picture(rect.width, rect.height) * MAX_PICTURES);
        let (mut frames, mut flashes) = (Vec::new(), Vec::new());
        for k in choose(self.kept, most) {
            let Some(mut frame) = picture::crop(&first, rect) else { continue };
            // The part of what it changed that the clip shows.
            if let Some((at, pixels)) = &k.patch
                && let Some(shown) = intersection(*at, rect)
                && let Some(part) =
                    picture::crop(pixels, Rect::new(shown.x - at.x, shown.y - at.y, shown.width, shown.height))
            {
                paste(&mut frame, &part, shown.x - rect.x, shown.y - rect.y);
            }
            if k.flash {
                flashes.push(frames.len());
            }
            frames.push((k.time, frame));
        }
        Some(Clip { frames, rect, seen: self.seen, seconds: seconds.max(self.last), flashes })
    }
}

/// Leaves in `kept` the first frame and the newest (the next one says whether it was a flash), the
/// frames settled on, the flashes, a frame of each movement large enough to see ([`movements`]),
/// and the others at least `step` apart in how much changed; says where the last of those is.
fn space(kept: &mut Vec<Kept>, step: u64) -> u64 {
    let middles: Vec<usize> = movements(kept).into_iter().map(|(_, middle)| middle).collect();
    let newest = kept.len().saturating_sub(1);
    let (mut anchor, mut i) = (0, 0);
    kept.retain(|k| {
        let (index, spaced) = (i, i == 0 || k.at - anchor >= step);
        i += 1;
        if spaced {
            anchor = k.at;
        }
        spaced || k.marked() || index == newest || middles.contains(&index)
    });
    anchor
}

/// Each movement in `kept` large enough to see (a menu opening, a list scrolling, a bar filling: the
/// frames between two that stay, of one movement): how much changed over it, from the frame before
/// it to the frame after, and its frame nearest halfway through that.
fn movements(kept: &[Kept]) -> Vec<(u64, usize)> {
    let last = kept.len().saturating_sub(1);
    let mut found = Vec::new();
    let mut i = 1;
    while i < last {
        let start = i;
        while i < last && !kept[i].marked() && kept[i].movement == kept[start].movement {
            i += 1;
        }
        if i == start {
            i += 1;
            continue;
        }
        let (from, to) = (kept[start - 1].at, kept[i].at);
        let half = from.midpoint(to);
        // Not the screen back as it was before a flash: shown already.
        let middle = (start..i).filter(|&j| !kept[j].back).min_by_key(|&j| kept[j].at.abs_diff(half));
        if let Some(middle) = middle.filter(|_| to - from >= REST_MIN) {
            found.push((to - from, middle));
        }
    }
    found
}

impl Kept {
    /// Whether it stays whatever changed around it: settled on, or a flash.
    fn marked(&self) -> bool {
        self.rest || self.flash
    }
}

/// Takes frames out of `kept` until `most` are left, never the first or the last: movements' frames
/// first, then frames settled on and flashes; each time the one nearest its neighbours in time (of a
/// hundred flickers, some from all along).
fn thin(kept: &mut Vec<Kept>, most: usize) {
    while kept.len() > most.max(2) {
        let between = |i: usize| kept[i + 1].time - kept[i - 1].time;
        let inner = 1..kept.len() - 1;
        let marked = !inner.clone().any(|i| !kept[i].marked());
        let candidates = inner.filter(|&i| kept[i].marked() == marked);
        let Some(nearest) = candidates.min_by(|&a, &b| between(a).total_cmp(&between(b))) else { return };
        kept.remove(nearest);
    }
}

/// The `most` frames of `kept` that tell what happened: the first, the last, the frames settled on,
/// the flashes, and of the others those nearest to evenly apart in how much changed.
fn choose(mut kept: Vec<Kept>, most: usize) -> Vec<Kept> {
    if kept.len() <= most {
        return kept;
    }
    let last = kept.len() - 1;
    let marked = (1..last).filter(|&i| kept[i].marked()).count();
    // More than fit: some from all along, and nothing else between the ends.
    if marked + 2 >= most {
        kept.retain({
            let mut i = 0;
            move |k| {
                i += 1;
                i == 1 || i == last + 1 || k.marked()
            }
        });
        thin(&mut kept, most);
        return kept;
    }
    let room = most - 2 - marked;
    // Each movement shows, halfway through: the largest first.
    let mut movements = movements(&kept);
    movements.sort_by_key(|&(size, _)| std::cmp::Reverse(size));
    let mut chosen: Vec<usize> = movements.into_iter().take(room).map(|(_, middle)| middle).collect();
    // Then the others evenly apart in how much changed.
    let (total, more) = (kept[last].at, room - chosen.len());
    for slot in 1..=more {
        let wanted = total * slot as u64 / (more as u64 + 1);
        let free = (1..last).filter(|i| !kept[*i].marked() && !kept[*i].back && !chosen.contains(i));
        chosen.extend(free.min_by_key(|&i| kept[i].at.abs_diff(wanted)));
    }
    let mut i = 0;
    kept.retain(|k| {
        i += 1;
        i == 1 || i == last + 1 || k.marked() || chosen.contains(&(i - 1))
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

/// The most frames `width` × `height` one picture holds, at the least size a frame is shown at
/// ([`MIN_SCALE`] of it, or smaller when one alone doesn't fit).
pub fn per_picture(width: u32, height: u32) -> usize {
    let (_, alone, _) = grid(1, width, height);
    let scale = (f64::from(alone) / f64::from(width)).min(MIN_SCALE);
    let (w, h) = ((f64::from(width) * scale) as u32, (f64::from(height) * scale) as u32);
    let columns = (PICTURE_SIDE - GAP) / (w + GAP);
    let rows = (PICTURE_SIDE - GAP) / (BAND + h + GAP);
    (columns * rows).max(1) as usize
}

/// How `n` frames `width` × `height` go in one picture with each as large as it can be (never larger
/// than recorded), and of those layouts the most compact: its columns, and a frame's size in it.
fn grid(n: u32, width: u32, height: u32) -> (u32, u32, u32) {
    let layout = |columns: u32| {
        let rows = n.div_ceil(columns);
        let across = PICTURE_SIDE.saturating_sub(GAP * (columns + 1)) as f64 / (columns * width) as f64;
        let down = PICTURE_SIDE.saturating_sub(GAP * (rows + 1) + BAND * rows) as f64 / (rows * height) as f64;
        let scale = across.min(down).min(1.0);
        let (w, h) = (((width as f64 * scale) as u32).max(1), ((height as f64 * scale) as u32).max(1));
        (w, h, (GAP + columns * (w + GAP)).max(GAP + rows * (BAND + h + GAP)))
    };
    let columns = (1..=n.max(1))
        .max_by(|&a, &b| {
            let ((wa, _, side_a), (wb, _, side_b)) = (layout(a), layout(b));
            wa.cmp(&wb).then(side_b.cmp(&side_a))
        })
        .unwrap_or(1);
    let (w, h, _) = layout(columns);
    (columns, w, h)
}

/// How many decimals frames' times are written with: two, or three when two frames would read the
/// same (a 120 Hz screen shows a frame every 8 ms).
fn digits(times: impl Iterator<Item = f32>) -> usize {
    let hundredths: Vec<i64> = times.map(|t| (f64::from(t) * 100.0).round() as i64).collect();
    if hundredths.windows(2).any(|t| t[0] == t[1]) { 3 } else { 2 }
}

/// How many pictures a clip's frames take ([`per_picture`]).
fn pictures_for(clip: &Clip) -> usize {
    clip.frames.first().map_or(0, |(_, f)| clip.frames.len().div_ceil(per_picture(f.width, f.height)))
}

/// A clip's frames in pictures, as few as hold them at the least size a frame is shown at, spread
/// evenly over them. In each, left to right then top to bottom, each frame under its number (counted
/// on from one picture to the next) and its time (`2  0.35s`), all of them the same size: as large as
/// the fullest picture lets them be, never larger than recorded.
pub fn pictures(clip: &Clip) -> Vec<Frame> {
    let Some((width, height)) = clip.frames.first().map(|(_, f)| (f.width, f.height)) else { return Vec::new() };
    let (n, count) = (clip.frames.len(), pictures_for(clip));
    let (columns, w, h) = grid(n.div_ceil(count) as u32, width, height);
    let digits = digits(clip.frames.iter().map(|(t, _)| *t));
    let mut frames = clip.frames.iter().enumerate();
    (0..count)
        .map(|picture| {
            // The first ones take one more, when they don't divide evenly.
            let here = (n / count + usize::from(picture < n % count)) as u32;
            let rows = here.div_ceil(columns);
            let mut sheet =
                Frame::filled(GAP + columns.min(here) * (w + GAP), GAP + rows * (BAND + h + GAP), BACKGROUND);
            for (place, (i, (time, frame))) in frames.by_ref().take(here as usize).enumerate() {
                let (column, row) = (place as u32 % columns, place as u32 / columns);
                let (x, y) = (GAP + column * (w + GAP), GAP + row * (BAND + h + GAP));
                let label = format!("{}  {time:.digits$}s", i + 1);
                write(&mut sheet, x + 2, y + (BAND - GLYPH.1 * 2) / 2, &label, 2);
                let tile =
                    if (w, h) == (frame.width, frame.height) { frame.clone() } else { picture::scaled(frame, w, h) };
                paste(&mut sheet, &tile, x, y + BAND);
            }
            sheet
        })
        .collect()
}

/// What's said after a clip's pictures, put together once the aside is sent (when what it's of has
/// its name: a pin's number can change): when each frame showed, how many the screen showed, over how
/// long, which it showed for an instant only, and in how many pictures.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    times: Vec<f32>,
    seen: usize,
    seconds: f32,
    flashes: Vec<usize>,
    pictures: usize,
}

impl Clip {
    pub fn summary(&self) -> Summary {
        Summary {
            times: self.frames.iter().map(|(t, _)| *t).collect(),
            seen: self.seen,
            seconds: self.seconds,
            flashes: self.flashes.clone(),
            pictures: pictures_for(self),
        }
    }
}

impl Summary {
    /// Said after the pictures: how many frames, of what (`place`: "in Safari", "on the screen"), over
    /// how long, in how many pictures, when each showed, how to read them, and
    /// which the screen showed for an instant only.
    pub fn caption(&self, place: &str) -> String {
        let digits = digits(self.times.iter().copied());
        let times: Vec<String> = self.times.iter().map(|t| format!("{t:.digits$}")).collect();
        match self.times.len() {
            0 | 1 => format!("**Clip:** nothing {place} changed in {:.1} s.", self.seconds),
            n => {
                let of = if self.seen > n { format!(" of the {} it showed", self.seen) } else { String::new() };
                let pictures = match self.pictures {
                    0 | 1 => String::new(),
                    count => format!(" in {count} pictures,"),
                };
                let numbers: Vec<String> = self.flashes.iter().map(|i| (i + 1).to_string()).collect();
                let flashes = match numbers.as_slice() {
                    [] => String::new(),
                    [one] => format!(" Frame {one} showed for only an instant."),
                    [rest @ .., last] => format!(" Frames {} and {last} showed for only an instant.", rest.join(", ")),
                };
                format!(
                    "**Clip:** {n} frames{of}, what changed {place} over {:.1} s,{pictures} left to right then top to \
                     bottom, at {} s.{flashes}",
                    self.seconds,
                    times.join(", ")
                )
            }
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
        assert_eq!(clip.flashes, [1]);
        assert_eq!(
            clip.summary().caption("on the screen"),
            "**Clip:** 3 frames, what changed on the screen over 2.0 s, left to right then top to bottom, at 0.00, \
             1.00, 1.02 s. Frame 2 showed for only an instant."
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
            // The frames kept, the newest, and two of the movement going on.
            assert!(recorder.len() <= KEPT + 3);
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
        let said = clip.summary().caption("in Safari");
        let flash = times.iter().position(|&t| t == 2.5).unwrap() + 1;
        assert!(said.starts_with("**Clip:** 12 frames of the 300 it showed, what changed in Safari over 5.0 s, left"));
        assert!(said.ends_with(&format!(" s. Frame {flash} showed for only an instant.")), "{said}");
    }

    #[test]
    fn keeps_each_screen_the_app_settled_on() {
        let mut recorder = Recorder::new();
        let mut frame = screen(100, 0x2563eb);
        recorder.push(0.0, frame.clone());
        let mut push = |recorder: &mut Recorder, time: f32, change: &dyn Fn(&mut Frame)| {
            change(&mut frame);
            recorder.push(time, frame.clone());
        };
        // A row selected, then the block turns green: each holds a second.
        push(&mut recorder, 1.0, &|f| f.fill(Rect::new(20, 40, 200, 20), 0x334155));
        push(&mut recorder, 2.0, &|f| f.fill(Rect::new(100, 100, 40, 20), 0x16a34a));
        // A tooltip, half a second: taken back, but held, so not a flash.
        push(&mut recorder, 2.4, &|f| f.fill(Rect::new(20, 250, 100, 30), 0x111111));
        push(&mut recorder, 2.9, &|f| f.fill(Rect::new(20, 250, 100, 30), 0xffffff));
        // A menu fades in over a quarter of a second, and stays open.
        for i in 0..15u32 {
            let level = 0xff - (i + 1) * 0x08;
            push(&mut recorder, 3.0 + i as f32 / 60.0, &|f| f.fill(Rect::new(250, 150, 100, 100), level * 0x010101));
        }
        // A caret blinks, every half second: never one settled on.
        for i in 0..3 {
            let color = if i % 2 == 0 { 0x000000 } else { 0xffffff };
            push(&mut recorder, 4.5 + i as f32 * 0.5, &|f| f.fill(Rect::new(300, 20, 2, 20), color));
        }
        // And a slide, a frame at a time, to the end.
        for i in 0..60u32 {
            push(&mut recorder, 6.0 + i as f32 / 60.0, &|f| {
                f.fill(Rect::new(0, 200, 400, 40), 0xffffff);
                f.fill(Rect::new(i * 4, 200, 100, 40), 0xea580c);
            });
        }
        let menu_open = 3.0 + 14.0 / 60.0;
        let rests: Vec<f32> = recorder.kept.iter().filter(|k| k.rest).map(|k| k.time).collect();
        assert_eq!(rests, [1.0, 2.0, 2.4, menu_open]);
        assert!(!recorder.kept.iter().any(|k| k.flash), "the tooltip held");
        let clip = recorder.finish(8.0).unwrap();
        let times: Vec<f32> = clip.frames.iter().map(|(t, _)| *t).collect();
        assert!(clip.frames.len() <= MAX_FRAMES && times.windows(2).all(|t| t[0] < t[1]), "{times:?}");
        for time in [0.0, 1.0, 2.0, 2.4, menu_open, 6.0 + 59.0 / 60.0] {
            assert!(times.contains(&time), "{time} went: {times:?}");
        }
        // Each as it was: the tooltip there, the menu fully open.
        let at = |time: f32, x: u32, y: u32| {
            let frame = &clip.frames[times.iter().position(|&t| t == time).unwrap()].1;
            frame.rgb(x - clip.rect.x, y - clip.rect.y)
        };
        assert_eq!((at(2.4, 30, 260), at(menu_open, 300, 200)), (0x111111, 0x878787));
    }

    #[test]
    fn a_flash_and_a_small_movement_after_a_long_scroll_stay() {
        let mut recorder = Recorder::new();
        // Stripes scroll for two seconds (all of it changing), and settle.
        let stripes = |offset: u32| {
            let mut frame = Frame::filled(W, H, 0xffffff);
            for y in (0..200).filter(|y| (y + offset) % 16 < 8) {
                frame.fill(Rect::new(0, y, W, 1), 0x222222);
            }
            frame
        };
        for i in 0..120 {
            recorder.push(i as f32 / 60.0, stripes(i * 3));
        }
        let settled = stripes(357);
        // A toast for one frame, then a bar fills, a little each frame.
        let mut toast = settled.clone();
        toast.fill(Rect::new(100, 240, 120, 30), 0x16a34a);
        recorder.push(3.0, toast);
        recorder.push(3.0 + 1.0 / 60.0, settled.clone());
        for i in 1..=60 {
            let mut bar = settled.clone();
            bar.fill(Rect::new(20, 285, i * 5, 6), 0x2563eb);
            recorder.push(3.5 + i as f32 / 60.0, bar);
        }
        let clip = recorder.finish(5.0).unwrap();
        let times: Vec<f32> = clip.frames.iter().map(|(t, _)| *t).collect();
        let toast = times.iter().position(|&t| t == 3.0).expect("the toast went");
        assert_eq!(clip.flashes, [toast], "{times:?}");
        // Not the screen back as it was after it: shown already, before it.
        assert!(!times.contains(&(3.0 + 1.0 / 60.0)), "{times:?}");
        // A third to two thirds through it.
        assert!(times.iter().any(|&t| t > 3.8 && t < 4.2), "the bar filling went: {times:?}");
    }

    #[test]
    fn stays_within_its_frames_however_long() {
        // A caret blinks for a minute: nothing settled on, nothing kept of it but its ends.
        let mut recorder = Recorder::new();
        for i in 0..120 {
            let color = if i % 2 == 0 { 0x000000 } else { 0xffffff };
            let mut frame = screen(100, 0x2563eb);
            frame.fill(Rect::new(300, 20, 2, 20), color);
            recorder.push(i as f32 * 0.5, frame);
            assert!(recorder.len() <= KEPT + 3);
        }
        assert!(recorder.kept.iter().all(|k| !k.marked()));
        // A hundred screens, each held half a second: some from all along.
        let mut recorder = Recorder::new();
        for i in 0..100u32 {
            recorder.push(i as f32 * 0.5, screen(i * 3, 0x2563eb));
            assert!(recorder.len() <= KEPT + 3);
        }
        let clip = recorder.finish(50.0).unwrap();
        let times: Vec<f32> = clip.frames.iter().map(|(t, _)| *t).collect();
        assert_eq!((times.len(), times[0], times[MAX_FRAMES - 1]), (MAX_FRAMES, 0.0, 49.5));
        assert!(times.windows(2).all(|t| t[1] - t[0] < 10.0), "{times:?}");
    }

    #[test]
    fn each_movement_shows() {
        let kept = |time: f32, at: u64, rest: bool| Kept {
            time,
            at,
            flash: false,
            rest,
            movement: 0,
            back: false,
            patch: None,
        };
        // A long scroll to a screen it settled on, then a small bar filling: room for two between.
        let mut frames: Vec<Kept> = (0..10).map(|i| kept(i as f32 / 16.0, i * 1000, false)).collect();
        frames.push(kept(1.0, 10_000, true));
        frames.extend((1..4).map(|i| kept(1.0 + i as f32 / 4.0, 10_000 + i * 100, false)));
        frames.push(kept(2.0, 10_400, false));
        let times: Vec<f32> = choose(frames, 5).iter().map(|k| k.time).collect();
        // Halfway through each, though the scroll changed far more.
        assert_eq!(times, [0.0, 0.3125, 1.0, 1.5, 2.0]);
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
        assert_eq!(clip.summary().caption("in Xcode"), "**Clip:** nothing in Xcode changed in 4.5 s.");
        assert!(Recorder::new().finish(1.0).is_none());
        // A frame of another size (the display changed) isn't taken.
        let mut recorder = Recorder::new();
        recorder.push(0.0, Frame::filled(10, 10, 0));
        recorder.push(0.1, Frame::filled(20, 10, 0xffffff));
        assert_eq!(recorder.len(), 1);
    }

    /// With `ASIDES_CLIP_DUMP=<dir>`, writes `clip-1.png` there (and on): a block sliding across a card
    /// and flashing once, as a clip's pictures, to look at, and what's said after them, `clip.txt`.
    #[test]
    fn dumps_a_clip() {
        let Some(dir) = std::env::var_os("ASIDES_CLIP_DUMP").map(std::path::PathBuf::from) else { return };
        let mut recorder = Recorder::new();
        for i in 0..240u32 {
            let mut frame = Frame::filled(900, 500, 0xf3f4f6);
            frame.fill(Rect::new(60, 120, 780, 220), 0xffffff);
            let color = if i == 130 { 0xdc2626 } else { 0x2563eb };
            frame.fill(Rect::new(80 + i * 2, 200, 160, 56), color);
            recorder.push(i as f32 / 60.0, frame);
        }
        let clip = recorder.finish(4.0).unwrap();
        for (i, sheet) in pictures(&clip).iter().enumerate() {
            std::fs::write(dir.join(format!("clip-{}.png", i + 1)), picture::png(sheet)).unwrap();
        }
        std::fs::write(dir.join("clip.txt"), clip.summary().caption("on the screen")).unwrap();
    }

    #[test]
    fn lays_the_frames_out_in_pictures() {
        let clip = |n: usize, width: u32, height: u32| Clip {
            frames: (0..n)
                .map(|i| (i as f32 * 0.25, Frame::filled(width, height, 0x100000 * (i as u32 + 1))))
                .collect(),
            rect: Rect::new(0, 0, width, height),
            seen: n,
            seconds: 2.0,
            flashes: Vec::new(),
        };
        // Three as recorded, in the most compact picture: two and one.
        let sheets = pictures(&clip(3, 300, 200));
        let sheet = &sheets[0];
        assert_eq!(sheets.len(), 1);
        assert_eq!((sheet.width, sheet.height), (GAP + 2 * (300 + GAP), GAP + 2 * (BAND + 200 + GAP)));
        assert_eq!(sheet.rgb(GAP, GAP + BAND), 0x100000);
        assert_eq!(sheet.rgb(GAP + 310, GAP + BAND + 199), 0x200000);
        assert_eq!(sheet.rgb(GAP, GAP + BAND + 200 + GAP + BAND), 0x300000);
        assert_eq!(sheet.rgb(0, 0), BACKGROUND);
        // Each under its number and time: the 1's stem is in its band, its corner isn't.
        let label = (GAP + 2, GAP + (BAND - 14) / 2);
        assert_eq!((sheet.rgb(label.0 + 2 * 2, label.1), sheet.rgb(label.0, label.1)), (INK, BACKGROUND));
        // Nine go three by three, still as recorded.
        let sheets = pictures(&clip(9, 300, 200));
        assert_eq!((sheets.len(), sheets[0].width, sheets[0].height), (1, GAP + 3 * 310, GAP + 3 * (BAND + 200 + GAP)));
        // Wide ones that wouldn't fit one picture readably take two, as recorded, numbered on: the
        // second starts at 3 (whose top is a bar).
        let wide = clip(4, 1400, 500);
        let sheets = pictures(&wide);
        assert_eq!((per_picture(1400, 500), sheets.len()), (3, 2));
        for sheet in &sheets {
            assert_eq!((sheet.width, sheet.height), (GAP + 1400 + GAP, GAP + 2 * (BAND + 500 + GAP)));
        }
        assert_eq!((sheets[1].rgb(GAP, GAP + BAND), sheets[1].rgb(label.0, label.1)), (0x300000, INK));
        assert!(wide.summary().caption("on the screen").contains(" over 2.0 s, in 2 pictures, left to right"));
        // Two frames 8 ms apart (a 120 Hz screen) don't read the same.
        let fast =
            Clip { frames: [0.0, 1.0, 1.004].map(|t| (t, Frame::filled(300, 200, 0))).to_vec(), ..clip(3, 300, 200) };
        assert!(fast.summary().caption("on the screen").ends_with(" at 0.000, 1.000, 1.004 s."));
        // Spread evenly, never much smaller than recorded: twelve halves of a screen, four a picture.
        let sheets = pictures(&clip(12, 900, 700));
        assert_eq!((per_picture(900, 700), sheets.len()), (4, 3));
        let tile = (sheets[0].width - 3 * GAP) / 2;
        assert!(tile as f64 >= 900.0 * MIN_SCALE && sheets.iter().all(|s| s.width == sheets[0].width), "{tile}");
        // A whole screen alone a picture, as large as one fits; one more than divides evenly.
        let sheets = pictures(&clip(5, 2000, 1250));
        assert_eq!((per_picture(2000, 1250), sheets.len()), (1, 5));
        assert_eq!(sheets[0].width, PICTURE_SIDE);
        let sheets = pictures(&clip(5, 900, 700));
        assert_eq!(sheets.iter().map(|s| s.height > GAP + BAND + 700).collect::<Vec<_>>(), [true, false]);
        for sheet in sheets.iter().chain(&pictures(&clip(12, 2000, 1250))) {
            assert!(sheet.width <= PICTURE_SIDE && sheet.height <= PICTURE_SIDE, "{} × {}", sheet.width, sheet.height);
        }
        assert!(pictures(&clip(0, 300, 200)).is_empty());
    }
}
