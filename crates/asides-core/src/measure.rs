//! Measuring what's on screen, the way PixelSnap does: from a point, the distance to the nearest
//! edge in each direction; inside a dragged area, the bounds of the object it surrounds; near a
//! guide, the edge it should snap to. Everything works on a frozen frame (a screenshot) in pixels,
//! origin top-left. An edge is where the color changes sharply: a pixel differs from the one before
//! it, or from the one two before (an edge anti-aliasing spread over two pixels), by more than the
//! tolerance, visibly, and all at once. Slow changes aren't edges, however far they go: gradients,
//! soft shadows, and what glass and frosted materials blur behind them, which take many pixels to
//! change even where they change fast. Nor are changes the eye can't see, however far apart the
//! channels are: a screen shows colors sRGB can't, and in sRGB their clipped channels jump by
//! several levels from one pixel to the next. A faint change is an edge only where it goes on along
//! a contour, the further the fainter (and further still where it doesn't stand out from grain
//! around it), or touches one that does, as the eye sees a faint outline but not a speck: grain, what
//! compression left and dust in a photo aren't edges, a dot, a faint card's border and its rounded
//! corners are. Raising the tolerance also looks through faint borders and hard shadows.

use std::collections::HashMap;
use std::ops::{Range, RangeInclusive};
use std::sync::OnceLock;

/// A screenshot as straight RGBA, row after row, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A rectangle in pixels (or points, depending on where it's used); `x`/`y` is its top-left corner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    /// The rectangle spanned by two corners, in either order, inclusive of both.
    pub fn from_corners(a: (u32, u32), b: (u32, u32)) -> Rect {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Rect { x: x0, y: y0, width: x1 - x0 + 1, height: y1 - y0 + 1 }
    }

    pub fn right(&self) -> u32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Whether the two overlap or come within `gap` of each other.
    pub fn near(&self, other: &Rect, gap: u32) -> bool {
        self.x <= other.right() + gap
            && other.x <= self.right() + gap
            && self.y <= other.bottom() + gap
            && other.y <= self.bottom() + gap
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Rect { x, y, width: self.right().max(other.right()) - x, height: self.bottom().max(other.bottom()) - y }
    }

    /// The part of this rectangle inside `width` × `height`.
    pub fn clamp_to(&self, width: u32, height: u32) -> Rect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        Rect { x, y, width: self.right().min(width) - x, height: self.bottom().min(height) - y }
    }
}

/// How different two neighbouring pixels must be to count as an edge, in percent of the full
/// channel range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tolerance(pub u8);

impl Tolerance {
    /// Low enough to see a light gray card on white (#F3F4F6 differs by 12 of 255).
    pub const DEFAULT: Tolerance = Tolerance(2);
    /// High enough to look through hard shadows and faint borders (PixelSnap's tolerance mode).
    pub const HIGH: Tolerance = Tolerance(30);
    pub const MAX: u8 = 100;

    /// The largest per-channel difference still seen as the same color.
    pub fn threshold(self) -> u8 {
        (self.0.min(Self::MAX) as u32 * 255 / 100) as u8
    }

    pub fn raised(self, by: u8) -> Tolerance {
        Tolerance(self.0.saturating_add(by).min(Self::MAX))
    }

    pub fn lowered(self, by: u8) -> Tolerance {
        Tolerance(self.0.saturating_sub(by))
    }

    /// PixelSnap's names for the tolerance: low sees the faintest edges, high looks through shadows.
    pub fn level(self) -> &'static str {
        match self.0 {
            0..=1 => "Low",
            2..=15 => "Medium",
            _ => "High",
        }
    }

    /// The tolerance a level name stands for (the overlay toolbar's choices).
    pub fn of_level(level: &str) -> Option<Tolerance> {
        match level {
            "Low" => Some(Tolerance(1)),
            "Medium" => Some(Tolerance::DEFAULT),
            "High" => Some(Tolerance::HIGH),
            _ => None,
        }
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance::DEFAULT
    }
}

/// A run of similar pixels along one axis: `start` is its first pixel, `end` one past its last,
/// so the distance between the edges on either side is `end - start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Whether the run reaches the frame's edge on either side instead of stopping at an edge.
    pub fn open(&self, limit: u32) -> (bool, bool) {
        (self.start == 0, self.end >= limit)
    }
}

/// From a point: the run of similar pixels through it, across and down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distance {
    pub x: u32,
    pub y: u32,
    pub horizontal: Span,
    pub vertical: Span,
}

/// Guides that stop a scan: vertical guides at these x boundaries stop horizontal scans, and
/// horizontal guides at these y boundaries stop vertical ones. A guide at `g` lies between
/// pixels `g - 1` and `g`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stops {
    pub x: Vec<u32>,
    pub y: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Along x: a vertical guide or line, positioned by its x.
    X,
    /// Along y: a horizontal guide or line, positioned by its y.
    Y,
}

/// A color as 0xRRGGBB.
pub type Rgb = u32;

impl Frame {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Frame> {
        (rgba.len() == width as usize * height as usize * 4).then_some(Frame { width, height, rgba })
    }

    /// A frame of one color, for tests and blank captures.
    pub fn filled(width: u32, height: u32, rgb: Rgb) -> Frame {
        let px = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255];
        Frame { width, height, rgba: px.repeat(width as usize * height as usize) }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }

    pub fn rgb(&self, x: u32, y: u32) -> Rgb {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let p = &self.rgba[i..i + 3];
        (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
    }

    /// Paints `rect` (clipped to the frame) in one color.
    pub fn fill(&mut self, rect: Rect, rgb: Rgb) {
        let rect = rect.clamp_to(self.width, self.height);
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let i = (y as usize * self.width as usize + x as usize) * 4;
                self.rgba[i..i + 4].copy_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255]);
            }
        }
    }

    /// A copy of `rect` (clipped to the frame) as a frame of its own.
    pub fn crop(&self, rect: Rect) -> Frame {
        let rect = rect.clamp_to(self.width, self.height);
        let mut rgba = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
        for y in rect.y..rect.bottom() {
            let start = (y as usize * self.width as usize + rect.x as usize) * 4;
            rgba.extend_from_slice(&self.rgba[start..start + rect.width as usize * 4]);
        }
        Frame { width: rect.width, height: rect.height, rgba }
    }
}

/// The largest difference between two colors' channels.
pub fn difference(a: Rgb, b: Rgb) -> u8 {
    [16, 8, 0].iter().map(|&s| (channel(a, s) - channel(b, s)).unsigned_abs()).max().unwrap_or(0) as u8
}

/// `#RRGGBB`.
pub fn hex(rgb: Rgb) -> String {
    format!("#{:06X}", rgb & 0xff_ffff)
}

fn channel(rgb: Rgb, shift: u32) -> i32 {
    ((rgb >> shift) & 0xff) as i32
}

/// How far apart two colors look: their distance in OKLab, where people start to notice a difference
/// around 0.02.
fn apparent_difference(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (oklab(a), oklab(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Two colors closer than this look the same: a 6-level step between light grays is about 0.018,
/// a red channel clipped at 0 that jumps to 12 in a sky blue 0.003.
const INVISIBLE: f32 = 0.008;
/// Channels further apart than this always look different (it takes about 40 levels in that sky
/// blue's red), so their colors aren't compared in OKLab: most edges, text's among them.
const SURELY_VISIBLE: u8 = 32;

fn oklab(rgb: Rgb) -> [f32; 3] {
    static LINEAR: OnceLock<[f32; 256]> = OnceLock::new();
    let linear = LINEAR.get_or_init(|| {
        std::array::from_fn(|v| {
            let c = v as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        })
    });
    let [r, g, b] = [16, 8, 0].map(|s| linear[channel(rgb, s) as usize]);
    let l = cbrt(0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b);
    let m = cbrt(0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b);
    let s = cbrt(0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b);
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// The cube root of `x` (0 to 1), to a hundred-thousandth: a guess from its bits, then two of
/// Newton's steps. Several times faster than `f32::cbrt`, and noisy screens need millions.
fn cbrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f32::from_bits(x.to_bits() / 3 + 709_921_077);
    y = (2.0 * y + x / (y * y)) / 3.0;
    (2.0 * y + x / (y * y)) / 3.0
}

/// A row or a column of a frame's pixels.
#[derive(Clone, Copy)]
struct Line<'a> {
    rgba: &'a [u8],
    /// Which of the frame's rows (or columns) it is, how many there are, and how far apart they are
    /// in bytes: where its first pixel is.
    index: u32,
    count: u32,
    apart: usize,
    /// How far apart its pixels are, in bytes, and how many it has.
    step: usize,
    len: u32,
}

impl<'a> Line<'a> {
    fn row(frame: &'a Frame, y: u32) -> Line<'a> {
        let apart = frame.width as usize * 4;
        Line { rgba: &frame.rgba, index: y, count: frame.height, apart, step: 4, len: frame.width }
    }

    fn column(frame: &'a Frame, x: u32) -> Line<'a> {
        let step = frame.width as usize * 4;
        Line { rgba: &frame.rgba, index: x, count: frame.width, apart: 4, step, len: frame.height }
    }

    /// The row (or column) `d` away from this one, when the frame has it.
    fn beside(&self, d: i32) -> Option<Line<'a>> {
        let index = self.index.checked_add_signed(d).filter(|&i| i < self.count)?;
        Some(Line { index, ..*self })
    }

    /// The column (or row) through pixel `i` of this row (or column).
    fn across(&self, i: u32) -> Line<'a> {
        Line { rgba: self.rgba, index: i, count: self.len, apart: self.step, step: self.apart, len: self.count }
    }

    /// Channel `c` (0 red, 1 green, 2 blue) of pixel `i`.
    fn level(&self, i: u32, c: usize) -> i32 {
        self.rgba[self.index as usize * self.apart + i as usize * self.step + c] as i32
    }

    fn rgb(&self, i: u32) -> Rgb {
        let p = &self.rgba[self.index as usize * self.apart + i as usize * self.step..][..3];
        (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
    }
}

/// The most pixels an edge's change spreads over where it's steepest (see [`spread`]): two or
/// three when it's sharp, up to a dozen when it's soft (a cloud in a painting, scaled up to the
/// screen). A gradient, a shadow or what glass blurs behind it takes more.
const EDGE_WIDTH: u32 = 12;

/// A change along a line, from pixel `from` to pixel `to`: the channel that changes most, which way
/// and by how much, the two colors, and the first and last pixels it spreads over (see [`spread`]).
#[derive(Clone, Copy)]
struct Change {
    from: u32,
    to: u32,
    channel: usize,
    rising: bool,
    rise: i32,
    colors: [Rgb; 2],
    extent: (u32, u32),
}

impl Change {
    /// Whether a change between `colors` on the line beside is like this one: changing its channel
    /// the same way by at least half as much (whichever of its own changes most: with grain, that
    /// differs from line to line when the three change alike), between colors nearer its own than
    /// they are to each other. A contour parts the same two things all along; grain changes as
    /// much, but from shade to shade at random.
    fn like(&self, colors: [Rgb; 2]) -> bool {
        let shift = 16 - 8 * self.channel as u32;
        let r = channel(colors[1], shift) - channel(colors[0], shift);
        let apart = difference(colors[0], self.colors[0]) as i32 + difference(colors[1], self.colors[1]) as i32;
        (if self.rising { r } else { -r }) * 2 >= self.rise && apart <= self.rise
    }
}

/// Changes this visible are edges even on a single pixel (a dot, a flower in a field); a fainter one
/// must go on along a contour, the further the fainter (see [`lines_needed`]), and as far as
/// [`THROUGH_GRAIN`] when it doesn't stand out from the grain around it (see [`stands_out`]).
const STRONG: f32 = 0.05;
/// Channels further apart than this always look that different, so their colors aren't compared.
const SURELY_STRONG: u8 = 64;
/// The most lines a change must cross to be an edge: one as faint as [`INVISIBLE`] (or fainter, its
/// channels far apart in a color that hides it).
const LONGEST: u32 = 7;
/// How many lines a faint change that doesn't stand out from the grain around it must cross: enough
/// that grain doesn't line up that far by chance, however dense (a line drawn on it goes on).
const THROUGH_GRAIN: u32 = 15;

/// How many lines (rows or columns in a row) a change whose colors look `seen` apart must cross to be
/// an edge: one when it's [`STRONG`], the more the fainter, up to [`LONGEST`].
fn lines_needed(seen: f32) -> u32 {
    ((STRONG / seen).ceil() as u32).clamp(1, LONGEST)
}

/// Whether the change from pixel `from` to pixel `to` (one or two further along a line) is an edge
/// (see [`edge_into`]).
fn is_edge(line: &Line, from: u32, to: u32, threshold: u8) -> bool {
    edge_into(line, to, threshold) == Some(from)
}

/// Where the edge ending at pixel `to` starts, when there's one: the change ending there (see
/// [`change_into`]), either [`STRONG`], going on along a contour (see [`goes_far`]), or touching one
/// that crosses the line (see [`joins`]).
fn edge_into(line: &Line, to: u32, threshold: u8) -> Option<u32> {
    let (change, lines) = change_into(line, to, threshold)?;
    (goes_far(line, change, lines, threshold) || joins(line, change, threshold)).then_some(change.from)
}

/// Whether the change ending at pixel `to` is [`STRONG`] or goes on along a contour, on its own.
fn goes_on(line: &Line, to: u32, threshold: u8) -> bool {
    change_into(line, to, threshold).is_some_and(|(change, lines)| goes_far(line, change, lines, threshold))
}

/// Whether a change that must cross `lines` lines (see [`lines_needed`]) goes on along a contour as
/// far as it must (see [`continues`]): across [`THROUGH_GRAIN`] when it doesn't stand out from the
/// grain around it (see [`stands_out`]), which is only worth knowing when it goes on across `lines`.
fn goes_far(line: &Line, change: Change, lines: u32, threshold: u8) -> bool {
    lines <= 1
        || (continues(line, change, lines, threshold)
            && (stands_out(line, change) || continues(line, change, THROUGH_GRAIN, threshold)))
}

/// Whether a faint change that doesn't go on along the lines beside it touches an edge across them
/// that does (see [`goes_on`]): ending on its line or the next, on the column (or row) through one
/// of its pixels. That's where a contour runs nearly along the line, so that it moves too far from a
/// line to the next to follow: the top of a faint card's rounded corner, the end of a hairline. A
/// speck of grain touches none.
fn joins(line: &Line, change: Change, threshold: u8) -> bool {
    (change.from..=change.to).any(|i| {
        let other = line.across(i);
        (line.index.max(1)..=(line.index + 1).min(other.len - 1)).any(|to| goes_on(&other, to, threshold))
    })
}

/// The change ending at pixel `to` (see [`change`]): from the pixel before it, or else from the one
/// before that (an edge anti-aliasing spread over two pixels).
fn change_into(line: &Line, to: u32, threshold: u8) -> Option<(Change, u32)> {
    change(line, to - 1, to, threshold).or_else(|| change(line, to.checked_sub(2)?, to, threshold))
}

/// The change from pixel `from` to pixel `to`, when it's by more than `threshold`, visible, and over
/// at most [`EDGE_WIDTH`] pixels (see [`spread`]), and how many lines it must cross to be an
/// edge for how far apart its two colors look (see [`lines_needed`]).
fn change(line: &Line, from: u32, to: u32, threshold: u8) -> Option<(Change, u32)> {
    let (a, b) = (line.rgb(from), line.rgb(to));
    let differ = difference(a, b);
    if differ <= threshold {
        return None;
    }
    let seen = if differ > SURELY_STRONG { f32::INFINITY } else { apparent_difference(a, b) };
    if differ <= SURELY_VISIBLE && seen < INVISIBLE {
        return None;
    }
    let c = (0..3).max_by_key(|&c| line.level(from, c).abs_diff(line.level(to, c))).unwrap_or(0);
    let rising = line.level(to, c) > line.level(from, c);
    let change = Change { from, to, channel: c, rising, rise: differ as i32, colors: [a, b], extent: (from, to) };
    let extent = spread(line, change, threshold)?;
    Some((Change { extent, ..change }, lines_needed(seen)))
}

/// How many pixels on either side of a faint change [`stands_out`] looks at, and how many times
/// the line's middling change there it must be.
const AROUND: u32 = 6;
const STANDS_OUT: i32 = 4;

/// Whether a change stands out from its line around it: by [`STANDS_OUT`] times as much as the line's
/// middling change from a pixel to the next over [`AROUND`] pixels, on each side of all it spreads
/// over (see [`spread`]: a soft edge's own pixels change alike). Grain (a painting's, a photo's, a
/// wallpaper's) changes about as much from every pixel to the next, and enough of it lines up by
/// chance over a few lines; on a flat or slowly changing background a faint change is all there is.
/// A faint line drawn on grain doesn't stand out from it either, but it goes on.
fn stands_out(line: &Line, change: Change) -> bool {
    let (first, last) = change.extent;
    let small = |&i: &u32| difference(line.rgb(i), line.rgb(i + 1)) as i32 * STANDS_OUT <= change.rise;
    // The middling step is small enough when more than half of them are (or there are none).
    let quiet = |steps: Range<u32>| steps.is_empty() || steps.clone().filter(small).count() * 2 > steps.len();
    quiet(first.saturating_sub(AROUND)..first) && quiet(last..(last + AROUND).min(line.len - 1))
}

/// The first and last pixels a change spreads over, when they're at most [`EDGE_WIDTH`] apart: as far
/// as the line keeps changing that way, along its channel, each pixel by at least half the largest
/// change (and half the threshold) from the one two before, so that a blur changing unevenly (1, 4, 2,
/// 3 levels a pixel) still counts as one change, and a soft edge's slower tail (a painted cloud's, over
/// as many pixels again) doesn't count against it. An edge on a gradient stops where the gradient's
/// slow change takes over.
fn spread(line: &Line, change: Change, threshold: u8) -> Option<(u32, u32)> {
    // The change from pixel `i` to pixel `j`, positive when it goes the edge's way.
    let rise = |i: u32, j: u32| {
        let r = line.level(j, change.channel) - line.level(i, change.channel);
        if change.rising { r } else { -r }
    };
    let mut peak = rise(change.from, change.to);
    let (mut first, mut last) = (change.from, change.to);
    while last - first <= EDGE_WIDTH {
        let least = (peak / 2).max((threshold as i32 + 1) / 2).max(1);
        let before = (first > 0).then(|| rise(first - 1, first + 1)).filter(|&r| r >= least);
        let after = (last + 1 < line.len).then(|| rise(last - 1, last + 1)).filter(|&r| r >= least);
        if before.is_none() && after.is_none() {
            return Some((first, last));
        }
        if let Some(r) = before {
            first -= 1;
            peak = peak.max(r);
        }
        if let Some(r) = after {
            last += 1;
            peak = peak.max(r);
        }
    }
    None
}

/// Whether a faint change goes on across the lines beside it, `lines` in all, each line's like the
/// one before (see [`near`]): following a contour, however it turns. An object's outline does, and
/// so does a hairline across the line, however faint; a speck of grain doesn't, nor a few in a row.
fn continues(line: &Line, change: Change, lines: u32, threshold: u8) -> bool {
    let mut found = 1;
    for d in [-1, 1] {
        let (mut on, mut at) = (*line, change);
        while found < lines {
            let Some(next) = on.beside(d) else { break };
            let Some(beside) = near(&next, at, threshold) else { break };
            (on, at, found) = (next, beside, found + 1);
        }
    }
    found >= lines
}

/// The first change on `line` (see [`change_into`]) like `change` on the line beside it (see
/// [`Change::like`]), ending from where it starts to a pixel after where it ends. That follows a
/// contour as steep as a pixel a line; one nearer the line's way crosses the lines across it more
/// steeply (see [`joins`]). A wider window let grain line up by chance.
fn near(line: &Line, change: Change, threshold: u8) -> Option<Change> {
    (change.from.max(1)..=(change.to + 1).min(line.len - 1))
        .find_map(|to| change_into(line, to, threshold).map(|(found, _)| found).filter(|f| change.like(f.colors)))
}

/// How many lines [`boundaries`] follows contours across at a time, and how many pixels along them:
/// enough that the lines and pixels around them a contour reaches are few, and few enough to stay in
/// the cache.
const BLOCK: u32 = 128;
const TILE: u32 = 1024;

/// A change [`change`] found, as [`Contours`] keeps it: the pixel it ends at and how many before it
/// starts (one or two), its two colors, which channel changes most, which way and by how much, how
/// many pixels it spreads over before and after its end, how many lines it must cross to be an edge
/// for how far apart its colors look, how many lines its contour goes on across before its own and
/// after it (as many as an edge needs at most), and whether that's as far as it must (see
/// [`goes_far`]).
#[derive(Clone, Copy)]
struct Found {
    to: u32,
    colors: [Rgb; 2],
    apart: u8,
    channel: u8,
    rising: bool,
    rise: u8,
    first: u8,
    last: u8,
    lines: u8,
    before: u8,
    after: u8,
    edge: bool,
}

impl Found {
    /// The first change like this one in `beside`, the changes on the line beside it in order (from
    /// `from` on, which it moves past those too far before), as [`near`] finds it.
    fn near<'a>(&self, beside: &'a [Found], from: &mut usize, len: u32) -> Option<&'a Found> {
        let (first, last) = ((self.to - self.apart as u32).max(1), (self.to + 1).min(len - 1));
        while beside.get(*from).is_some_and(|f| f.to + 2 < self.to) {
            *from += 1;
        }
        let change = self.change();
        beside[*from..].iter().take_while(|f| f.to <= last).find(|f| f.to >= first && change.like(f.colors))
    }

    fn is_edge(&self) -> bool {
        self.edge
    }

    /// The change it is, as [`change`] found it.
    fn change(&self) -> Change {
        let (to, channel, rising, rise) = (self.to, self.channel as usize, self.rising, self.rise as i32);
        let extent = (to - self.first as u32, to + self.last as u32);
        Change { from: to - self.apart as u32, to, channel, rising, rise, colors: self.colors, extent }
    }
}

/// The changes on some lines (rows or columns), one ending at a pixel at most (see [`change_into`]),
/// each with how far its contour goes on across the lines before and after it: what [`continues`]
/// follows from each change on its own, found for all of them at once, a line after another, one
/// way then the other. It also holds the lines and pixels around them that those contours reach.
struct Contours {
    rows: bool,
    lines: Range<u32>,
    positions: Range<u32>,
    /// Whether each pixel differs from the one before it on its line by more than the threshold,
    /// laid out like the frame (row by row).
    differs: Vec<bool>,
    /// The changes, a line after another, each line's in order; where each line's start.
    found: Vec<Found>,
    starts: Vec<usize>,
}

impl Contours {
    fn new(frame: &Frame, rows: bool, lines: Range<u32>, positions: Range<u32>, threshold: u8) -> Self {
        let (count, len) = if rows { (frame.height, frame.width) } else { (frame.width, frame.height) };
        let reach = THROUGH_GRAIN - 1;
        // Changes the other way look up those on the two lines before their own and the one after
        // (see [`Contours::touch`]), whose contours must be followed as far as they reach.
        let lines = lines.start.saturating_sub(reach + 2)..(lines.end + reach + 1).min(count);
        // A contour's change on each line ends within two pixels before its change on the line before,
        // or one after (see [`near`]); one two apart that ends just past `positions` counts.
        let positions = positions.start.saturating_sub(2 * reach)..(positions.end + 1 + reach).min(len);
        let mut contours =
            Contours { rows, lines, positions, differs: Vec::new(), found: Vec::new(), starts: Vec::new() };
        let (lines, positions) = (contours.lines.clone(), contours.positions.clone());
        // The pairs that differ, a frame row at a time (which vectorizes): one pixel apart, kept, and two.
        let mut differs = [vec![false; lines.len() * positions.len()], vec![false; lines.len() * positions.len()]];
        let mut bytes = vec![0u8; positions.len().max(lines.len()) * 4];
        let at = |x: u32, y: u32| (y as usize * frame.width as usize + x as usize) * 4;
        for (apart, differs) in [1, 2].into_iter().zip(differs.iter_mut()) {
            let first = positions.start.max(apart);
            let n = positions.end.saturating_sub(first) as usize;
            if rows {
                for (y, out) in lines.clone().zip(differs.chunks_mut(positions.len())) {
                    let (a, b) = (&frame.rgba[at(first - apart, y)..][..n * 4], &frame.rgba[at(first, y)..][..n * 4]);
                    differ(a, b, threshold, &mut bytes, &mut out[(first - positions.start) as usize..]);
                }
            } else {
                let n = lines.len() * 4;
                for (y, out) in
                    (first..positions.end).zip(differs.chunks_mut(lines.len()).skip((first - positions.start) as usize))
                {
                    let (a, b) =
                        (&frame.rgba[at(lines.start, y - apart)..][..n], &frame.rgba[at(lines.start, y)..][..n]);
                    differ(a, b, threshold, &mut bytes, out);
                }
            }
        }
        for l in lines.clone() {
            contours.starts.push(contours.found.len());
            let line = if rows { Line::row(frame, l) } else { Line::column(frame, l) };
            for to in positions.clone() {
                let i = contours.index(l, to);
                let Some((c, needed)) = [1, 2]
                    .into_iter()
                    .filter(|&apart| differs[apart as usize - 1][i])
                    .find_map(|apart| change(&line, to - apart, to, threshold))
                else {
                    continue;
                };
                contours.found.push(Found {
                    to,
                    colors: c.colors,
                    apart: (to - c.from) as u8,
                    channel: c.channel as u8,
                    rising: c.rising,
                    rise: c.rise as u8,
                    first: (to - c.extent.0) as u8,
                    last: (c.extent.1 - to) as u8,
                    lines: needed as u8,
                    before: 0,
                    after: 0,
                    edge: false,
                });
            }
        }
        contours.starts.push(contours.found.len());
        let [differs, _] = differs;
        contours.differs = differs;
        // How far each change's contour goes on before its line, a line after another from the first,
        // then after it, from the last.
        let starts = &contours.starts;
        for k in 1..lines.len() {
            let (done, rest) = contours.found.split_at_mut(starts[k]);
            let (before, mut from) = (&done[starts[k - 1]..], 0);
            for f in &mut rest[..starts[k + 1] - starts[k]] {
                if let Some(b) = f.near(before, &mut from, len) {
                    f.before = (b.before + 1).min(reach as u8);
                }
            }
        }
        for k in (0..lines.len().saturating_sub(1)).rev() {
            let (rest, done) = contours.found.split_at_mut(starts[k + 1]);
            let (after, mut from) = (&done[..starts[k + 2] - starts[k + 1]], 0);
            for f in &mut rest[starts[k]..] {
                if let Some(a) = f.near(after, &mut from, len) {
                    f.after = (a.after + 1).min(reach as u8);
                }
            }
        }
        // Which go on as far as they must.
        for (k, l) in lines.enumerate() {
            let line = if rows { Line::row(frame, l) } else { Line::column(frame, l) };
            for f in &mut contours.found[starts[k]..starts[k + 1]] {
                let (across, lines) = (1 + f.before as u32 + f.after as u32, f.lines as u32);
                f.edge = lines <= 1 || (across >= lines && (across >= THROUGH_GRAIN || stands_out(&line, f.change())));
            }
        }
        contours
    }

    /// Where pixel `at` of line `line` is in `differs`.
    fn index(&self, line: u32, at: u32) -> usize {
        let (l, p) = ((line - self.lines.start) as usize, (at - self.positions.start) as usize);
        if self.rows { l * self.positions.len() + p } else { p * self.lines.len() + l }
    }

    /// Whether pixel `at` of line `line` differs from the one before it by more than the threshold.
    fn differs(&self, line: u32, at: u32) -> bool {
        self.differs[self.index(line, at)]
    }

    /// The changes on line `line`, in order.
    fn on(&self, line: u32) -> &[Found] {
        let l = (line - self.lines.start) as usize;
        &self.found[self.starts[l]..self.starts[l + 1]]
    }

    /// Whether one of `lines` has an edge of its own (see [`Found::is_edge`]) ending at pixel `at` or
    /// the next: what [`joins`] looks for across a change on line `at` the other way.
    fn touch(&self, lines: RangeInclusive<u32>, at: u32) -> bool {
        lines.into_iter().any(|l| {
            let on = self.on(l);
            on[on.partition_point(|f| f.to < at)..].iter().take_while(|f| f.to <= at + 1).any(Found::is_edge)
        })
    }
}

/// Where `frame` has edges within `rows` and `columns`, into `out` (row by row): [`ACROSS`] at `(x, y)`
/// when there's one between pixels `x - 1` and `x` of row `y`, [`DOWN`] when there's one between pixels
/// `y - 1` and `y` of column `x`. That's an edge between them (see [`is_edge`]), or between pixels two
/// apart while the pair on the other side is alike: an edge that anti-aliasing spread over two pixels,
/// whichever way it's crossed. Contours are followed a [`BLOCK`] of rows and a [`TILE`] of columns at
/// a time, along both.
fn boundaries(frame: &Frame, rows: Range<u32>, columns: Range<u32>, threshold: u8, out: &mut [u8]) {
    let w = columns.len();
    for start in rows.clone().step_by(BLOCK as usize) {
        let block = start..(start + BLOCK).min(rows.end);
        for tile in columns.clone().step_by(TILE as usize) {
            let tile = tile..(tile + TILE).min(columns.end);
            let across = Contours::new(frame, true, block.clone(), tile.clone(), threshold);
            let down = Contours::new(frame, false, tile.clone(), block.clone(), threshold);
            for (contours, other, bit) in [(&across, &down, ACROSS), (&down, &across, DOWN)] {
                // This way's lines and the pixels along them: in the block and the area.
                let (lines, part, whole) = match bit {
                    ACROSS => (&block, &tile, &columns),
                    _ => (&tile, &block, &rows),
                };
                for l in lines.clone() {
                    // The edge is between `p - 1` and `p`.
                    let mut mark = |p: u32| {
                        if part.contains(&p) && p > whole.start {
                            let (x, y) = if bit == ACROSS { (p, l) } else { (l, p) };
                            out[(y - rows.start) as usize * w + (x - columns.start) as usize] |= bit;
                        }
                    };
                    let ends = part.start..(part.end + 1).min(whole.end);
                    for f in contours.on(l).iter().filter(|f| ends.contains(&f.to)) {
                        if !f.is_edge() && !other.touch(f.to - f.apart as u32..=f.to, l) {
                            continue;
                        }
                        if f.apart == 1 {
                            mark(f.to);
                            continue;
                        }
                        if f.to >= whole.start + 2 && !contours.differs(l, f.to - 1) {
                            mark(f.to);
                        }
                        if !contours.differs(l, f.to) {
                            mark(f.to - 1);
                        }
                    }
                }
            }
        }
    }
}

/// What [`boundaries`] marks: an edge before a pixel across its row, and down its column.
const ACROSS: u8 = 1;
const DOWN: u8 = 2;

/// Whether each pixel of `a` differs from the one at the same place in `b` by more than `threshold`,
/// into `out` (byte by byte first, into `bytes`, which vectorizes).
fn differ(a: &[u8], b: &[u8], threshold: u8, bytes: &mut [u8], out: &mut [bool]) {
    let n = a.len().min(b.len());
    for ((d, p), q) in bytes[..n].iter_mut().zip(&a[..n]).zip(&b[..n]) {
        *d = p.abs_diff(*q);
    }
    for (px, d) in bytes[..n].as_chunks::<4>().0.iter().zip(out.iter_mut()) {
        *d = px[0].max(px[1]).max(px[2]) > threshold;
    }
}

/// The run through pixel `at` of a line, up to the first edge on either side (see [`edge_into`]):
/// between neighbours, or between pixels two apart on the run (an edge anti-aliasing spread over two
/// pixels).
fn run(line: &Line, at: u32, threshold: u8) -> Span {
    let edge_into = |to: u32| edge_into(line, to, threshold);
    let mut end = at + 1;
    while end < line.len && !edge_into(end).is_some_and(|from| from + 1 == end || end >= at + 2) {
        end += 1;
    }
    // Going left, the edge ending at the pixel after `start` too.
    let (mut start, mut after) = (at, None);
    while start > 0 {
        let into = edge_into(start);
        if into == Some(start - 1) || (start < at && after == Some(start - 1)) {
            break;
        }
        (start, after) = (start - 1, into);
    }
    Span { start, end }
}

/// The distances from `(x, y)` to the nearest edge left, right, up and down, stopping at guides.
pub fn distance_at(frame: &Frame, x: u32, y: u32, tolerance: Tolerance, stops: &Stops) -> Option<Distance> {
    if !frame.contains(x, y) {
        return None;
    }
    let threshold = tolerance.threshold();
    let across = run(&Line::row(frame, y), x, threshold);
    let down = run(&Line::column(frame, x), y, threshold);
    let (left, right) = clip_to_stops(across.start, across.end, x, &stops.x);
    let (top, bottom) = clip_to_stops(down.start, down.end, y, &stops.y);
    Some(Distance { x, y, horizontal: Span { start: left, end: right }, vertical: Span { start: top, end: bottom } })
}

/// Shrinks the run `start..end` around `at` to the guides on either side of `at`.
fn clip_to_stops(mut start: u32, mut end: u32, at: u32, stops: &[u32]) -> (u32, u32) {
    for &g in stops {
        if g <= at && g > start {
            start = g;
        }
        if g > at && g < end {
            end = g;
        }
    }
    (start, end)
}

/// The color most of `rect`'s outline has: what an object drawn inside it sits on.
pub fn background(frame: &Frame, rect: Rect) -> Option<Rgb> {
    let rect = rect.clamp_to(frame.width, frame.height);
    if rect.is_empty() {
        return None;
    }
    let mut counts: HashMap<Rgb, u32> = HashMap::new();
    let mut count = |x: u32, y: u32| *counts.entry(frame.rgb(x, y)).or_default() += 1;
    for x in rect.x..rect.right() {
        count(x, rect.y);
        count(x, rect.bottom() - 1);
    }
    for y in rect.y..rect.bottom() {
        count(rect.x, y);
        count(rect.right() - 1, y);
    }
    counts.into_iter().max_by_key(|&(rgb, n)| (n, rgb)).map(|(rgb, _)| rgb)
}

/// Fills `out`, rows `w` wide, a band of rows at a time (`fill(first row, its rows)`): on several
/// threads when it's large enough to be worth them, each taking the next band when it's done with
/// one (a busy part of a picture takes longer).
fn by_rows<T: Send>(out: &mut [T], w: usize, fill: impl Fn(usize, &mut [T]) + Sync) {
    let h = out.len() / w.max(1);
    let threads = match out.len() {
        0..0x40000 => 1,
        _ => std::thread::available_parallelism().map_or(1, |n| n.get()).min(8),
    };
    if threads <= 1 || h < 2 {
        return fill(0, out);
    }
    let band = h.div_ceil(threads * 3);
    let bands = std::sync::Mutex::new(out.chunks_mut(band * w).enumerate());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let next = bands.lock().map(|mut b| b.next());
                    let Ok(Some((k, rows))) = next else { break };
                    fill(k * band, rows);
                }
            });
        }
    });
}

/// Which pixels of `rect` (row by row) are drawn on its background: those the background on its
/// outline can't reach without crossing a sharp change, stepping across and down. What the
/// background does on the way doesn't matter (a gradient, a soft shadow, what glass blurs behind
/// it); what's drawn on it has edges. The outline's background is its longest stretch without a
/// sharp change, and the stretches colored like it: what crosses the outline (a button the area
/// cuts into) is drawn on it.
///
/// The edges are those measuring finds from a point (see [`is_edge`]), between neighbours or over two
/// anti-aliased pixels (see [`boundaries`]), found a block of rows (then of columns) at a time, then
/// the background filled a stretch of a row at a time (a scanline fill).
fn content_map(frame: &Frame, rect: Rect, threshold: u8) -> Vec<bool> {
    let (w, h) = (rect.width as usize, rect.height as usize);
    let mut edges = vec![0u8; w * h];
    by_rows(&mut edges, w, |first, out| {
        let rows = rect.y + first as u32..rect.y + (first + out.len() / w) as u32;
        boundaries(frame, rows, rect.x..rect.right(), threshold, out);
    });
    // Across: `edge_across(y * w + x)`, between (x - 1, y) and (x, y); down: `edge_down(y * w + x)`,
    // between (x, y - 1) and (x, y).
    let edge_across = |i: usize| edges[i] & ACROSS != 0;
    let edge_down = |i: usize| edges[i] & DOWN != 0;
    let stride = frame.width as usize * 4;
    let mut reached = vec![false; w * h];
    let pixel = |x: usize, y: usize| {
        let i = (rect.y as usize + y) * stride + (rect.x as usize + x) * 4;
        [frame.rgba[i], frame.rgba[i + 1], frame.rgba[i + 2]]
    };
    let mut stack = outline_background(w, h, threshold, pixel, |a, b| {
        // Neighbours on the outline: across or down from one another.
        let (first, second) = if (a.1, a.0) < (b.1, b.0) { (a, b) } else { (b, a) };
        if first.1 == second.1 { edge_across(second.1 * w + second.0) } else { edge_down(second.1 * w + second.0) }
    });
    while let Some((x, y)) = stack.pop() {
        let row = y * w;
        if reached[row + x] {
            continue;
        }
        let (mut left, mut right) = (x, x);
        while left > 0 && !reached[row + left - 1] && !edge_across(row + left) {
            left -= 1;
        }
        while right + 1 < w && !reached[row + right + 1] && !edge_across(row + right + 1) {
            right += 1;
        }
        reached[row + left..=row + right].fill(true);
        // Seeds in the rows above and below: one per stretch that this one opens onto.
        for (other, boundary) in [(y.checked_sub(1), y), ((y + 1 < h).then_some(y + 1), y + 1)] {
            let Some(other) = other else { continue };
            let mut open_before = false;
            for cx in left..=right {
                let open = !reached[other * w + cx] && !edge_down(boundary * w + cx);
                if open && (!open_before || edge_across(other * w + cx)) {
                    stack.push((cx, other));
                }
                open_before = open;
            }
        }
    }
    reached.into_iter().map(|r| !r).collect()
}

/// The pixels on the outline of a `w` × `h` area that are its background: its longest stretch
/// without an edge (`edge` between two neighbours on it), the stretches whose average color is
/// within `threshold` of that one's, and any as long as a quarter of the outline (an object across
/// the area cuts the background in two, and behind glass the two halves differ).
fn outline_background(
    w: usize,
    h: usize,
    threshold: u8,
    pixel: impl Fn(usize, usize) -> [u8; 3],
    edge: impl Fn((usize, usize), (usize, usize)) -> bool,
) -> Vec<(usize, usize)> {
    // Clockwise from the top-left corner, each pixel once.
    let mut ring: Vec<(usize, usize)> = (0..w).map(|x| (x, 0)).collect();
    ring.extend((1..h).map(|y| (w - 1, y)));
    if h > 1 {
        ring.extend((0..w.saturating_sub(1)).rev().map(|x| (x, h - 1)));
    }
    if w > 1 {
        ring.extend((1..h.saturating_sub(1)).rev().map(|y| (0, y)));
    }
    let n = ring.len();
    let cuts: Vec<usize> = (0..n).filter(|&i| n > 1 && edge(ring[i], ring[(i + 1) % n])).map(|i| (i + 1) % n).collect();
    if cuts.is_empty() {
        return ring;
    }
    // Stretches between cuts (going round), each with its color summed.
    let stretches: Vec<Vec<(usize, usize)>> = cuts
        .iter()
        .enumerate()
        .map(|(k, &start)| {
            let end = cuts[(k + 1) % cuts.len()];
            let len = (end + n - start - 1) % n + 1;
            (0..len).map(|j| ring[(start + j) % n]).collect()
        })
        .collect();
    let average = |stretch: &[(usize, usize)]| {
        let mut sum = [0u64; 3];
        for &(x, y) in stretch {
            let p = pixel(x, y);
            (0..3).for_each(|c| sum[c] += p[c] as u64);
        }
        sum.map(|c| (c / stretch.len().max(1) as u64) as u8)
    };
    let longest = stretches.iter().max_by_key(|s| s.len()).expect("a cut makes a stretch");
    let background = average(longest);
    stretches
        .iter()
        .filter(|s| {
            s.len() == longest.len() || s.len() * 4 >= n || {
                let color = average(s);
                (0..3).all(|c| color[c].abs_diff(background[c]) <= threshold)
            }
        })
        .flatten()
        .copied()
        .collect()
}

/// The bounds of whatever `rect` surrounds: the smallest rectangle holding everything drawn on its
/// background (see [`content_map`]). `None` when nothing is.
pub fn snap_area(frame: &Frame, rect: Rect, tolerance: Tolerance) -> Option<Rect> {
    let rect = rect.clamp_to(frame.width, frame.height);
    if rect.is_empty() {
        return None;
    }
    bounds(&content_map(frame, rect, tolerance.threshold()), rect)
}

/// The smallest rectangle holding the content pixels of `rect` (`content`, row by row).
fn bounds(content: &[bool], rect: Rect) -> Option<Rect> {
    let w = rect.width as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, _) in content.iter().enumerate().filter(|(_, c)| **c) {
        let (x, y) = (i % w, i / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    (x0 != usize::MAX)
        .then(|| Rect::from_corners((rect.x + x0 as u32, rect.y + y0 as u32), (rect.x + x1 as u32, rect.y + y1 as u32)))
}

/// The separate objects inside `rect`: groups of pixels drawn on the background (see
/// [`content_map`]), each as its bounds, largest first. Groups within `gap` pixels of each other
/// are one object (the letters of a word, the parts of an icon), and specks smaller than `gap` on
/// both sides are left out.
pub fn objects(frame: &Frame, rect: Rect, tolerance: Tolerance, gap: u32) -> Vec<Rect> {
    let rect = rect.clamp_to(frame.width, frame.height);
    if rect.is_empty() {
        return Vec::new();
    }
    let (w, h) = (rect.width as usize, rect.height as usize);
    let content = content_map(frame, rect, tolerance.threshold());
    // Flood-fill each group of touching content pixels (diagonals too), keeping its bounds.
    let mut seen = vec![false; w * h];
    let mut groups: Vec<Rect> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !content[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            let (left, right, up, down) = (x > 0, x + 1 < w, y > 0, y + 1 < h);
            let neighbours = [
                left.then(|| i - 1),
                right.then(|| i + 1),
                up.then(|| i - w),
                down.then(|| i + w),
                (left && up).then(|| i - w - 1),
                (right && up).then(|| i - w + 1),
                (left && down).then(|| i + w - 1),
                (right && down).then(|| i + w + 1),
            ];
            for n in neighbours.into_iter().flatten() {
                if content[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        groups.push(Rect::from_corners(
            (rect.x + x0 as u32, rect.y + y0 as u32),
            (rect.x + x1 as u32, rect.y + y1 as u32),
        ));
    }
    // Merge groups that are close, until nothing changes.
    let mut merged = true;
    while merged {
        merged = false;
        let mut out: Vec<Rect> = Vec::new();
        for g in groups {
            match out.iter_mut().find(|o| o.near(&g, gap)) {
                Some(o) => {
                    *o = o.union(&g);
                    merged = true;
                }
                None => out.push(g),
            }
        }
        groups = out;
    }
    groups.retain(|g| g.width > gap || g.height > gap);
    groups.sort_by_key(|g| std::cmp::Reverse(g.width as u64 * g.height as u64));
    groups
}

/// `16:9` when a width and height make a simple ratio (both terms 32 or less), else `None`.
pub fn aspect_ratio(width: f64, height: f64) -> Option<String> {
    let (w, h) = (width.round() as u64, height.round() as u64);
    if w == 0 || h == 0 {
        return None;
    }
    let gcd = |mut a: u64, mut b: u64| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let g = gcd(w, h);
    let (a, b) = (w / g, h / g);
    (a <= 32 && b <= 32).then(|| format!("{a}:{b}"))
}

/// The edge nearest to `(x, y)` along `axis`, within `radius` pixels: for a vertical guide, the
/// x boundary where the row at `y` changes color; for a horizontal one, the y boundary where the
/// column at `x` does. A boundary `b` lies between pixels `b - 1` and `b`.
pub fn nearest_edge(frame: &Frame, axis: Axis, x: u32, y: u32, radius: u32, tolerance: Tolerance) -> Option<u32> {
    if !frame.contains(x, y) {
        return None;
    }
    let threshold = tolerance.threshold();
    let (at, line) = match axis {
        Axis::X => (x, Line::row(frame, y)),
        Axis::Y => (y, Line::column(frame, x)),
    };
    let edge = |b: u32| b > 0 && b < line.len && is_edge(&line, b - 1, b, threshold);
    // The boundaries `d` pixels away on either side of the pixel at `at`, closest first.
    (0..=radius).find_map(|d| [at.checked_sub(d), at.checked_add(d + 1)].into_iter().flatten().find(|&b| edge(b)))
}

/// How far the content inside `rect` sits from its sides: top, right, bottom, left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insets {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

/// What an element looks like inside `rect`: the color it's filled with, the most common color
/// drawn on it (usually its text), and how far that content sits from its sides (its padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub background: Rgb,
    pub foreground: Option<Rgb>,
    pub content: Option<Rect>,
    pub insets: Option<Insets>,
}

pub fn appearance(frame: &Frame, rect: Rect, tolerance: Tolerance) -> Option<Appearance> {
    let rect = rect.clamp_to(frame.width, frame.height);
    let bg = background(frame, rect)?;
    let content = content_map(frame, rect, tolerance.threshold());
    let mut counts: HashMap<Rgb, u32> = HashMap::new();
    let w = rect.width as usize;
    for (i, _) in content.iter().enumerate().filter(|(_, c)| **c) {
        *counts.entry(frame.rgb(rect.x + (i % w) as u32, rect.y + (i / w) as u32)).or_default() += 1;
    }
    let foreground = counts.into_iter().max_by_key(|&(rgb, n)| (n, rgb)).map(|(rgb, _)| rgb);
    let content = bounds(&content, rect);
    let insets = content.map(|c| Insets {
        top: c.y - rect.y,
        right: rect.right() - c.right(),
        bottom: rect.bottom() - c.bottom(),
        left: c.x - rect.x,
    });
    Some(Appearance { background: bg, foreground, content, insets })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgb = 0xffffff;
    const BLUE: Rgb = 0x2563eb;
    const GRAY: Rgb = 0xf3f4f6;

    /// A 200×100 white frame with a blue 60×20 button at (40, 30) and white text on it.
    fn button() -> Frame {
        let mut frame = Frame::filled(200, 100, WHITE);
        frame.fill(Rect::new(40, 30, 60, 20), BLUE);
        frame.fill(Rect::new(52, 36, 36, 8), WHITE);
        frame
    }

    #[test]
    fn measures_the_gap_between_edges() {
        let frame = button();
        // In the white space left of the button: from the frame's edge to the button.
        let d = distance_at(&frame, 10, 40, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.horizontal, Span { start: 0, end: 40 });
        assert_eq!(d.vertical, Span { start: 0, end: 100 });
        assert_eq!(d.horizontal.open(frame.width), (true, false));
        // On the button, beside the text: from the button's left edge to the text.
        let d = distance_at(&frame, 45, 40, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!((d.horizontal, d.vertical), (Span { start: 40, end: 52 }, Span { start: 30, end: 50 }));
        assert_eq!(d.horizontal.len(), 12);
        assert!(distance_at(&frame, 200, 0, Tolerance::DEFAULT, &Stops::default()).is_none());
    }

    #[test]
    fn tolerance_looks_through_shadows() {
        let mut frame = button();
        // A soft shadow under the button.
        frame.fill(Rect::new(40, 50, 60, 3), 0xe8e8e8);
        let low = distance_at(&frame, 70, 80, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(low.vertical, Span { start: 53, end: 100 });
        let high = distance_at(&frame, 70, 80, Tolerance::HIGH, &Stops::default()).unwrap();
        assert_eq!(high.vertical, Span { start: 50, end: 100 });
        assert_eq!(Tolerance(100).threshold(), 255);
        assert_eq!(Tolerance(0).threshold(), 0);
        assert_eq!(Tolerance(98).raised(5), Tolerance(100));
        assert_eq!(Tolerance(2).lowered(5), Tolerance(0));
    }

    #[test]
    fn guides_stop_the_scan() {
        let frame = button();
        let stops = Stops { x: vec![20, 5, 150], y: vec![60] };
        let d = distance_at(&frame, 10, 70, Tolerance::DEFAULT, &stops).unwrap();
        assert_eq!(d.horizontal, Span { start: 5, end: 20 });
        assert_eq!(d.vertical, Span { start: 60, end: 100 });
    }

    #[test]
    fn snaps_an_area_to_the_object_inside() {
        let frame = button();
        assert_eq!(snap_area(&frame, Rect::new(20, 10, 120, 70), Tolerance::DEFAULT), Some(Rect::new(40, 30, 60, 20)));
        // Inside the button, the text is the object.
        assert_eq!(snap_area(&frame, Rect::new(44, 33, 50, 14), Tolerance::DEFAULT), Some(Rect::new(52, 36, 36, 8)));
        assert_eq!(snap_area(&frame, Rect::new(0, 0, 30, 30), Tolerance::DEFAULT), None);
        // Areas reaching past the frame are clipped to it.
        assert_eq!(snap_area(&frame, Rect::new(30, 20, 900, 900), Tolerance::DEFAULT), Some(Rect::new(40, 30, 60, 20)));
        assert_eq!(Rect::from_corners((10, 8), (2, 3)), Rect::new(2, 3, 9, 6));
    }

    #[test]
    fn finds_the_nearest_edge_for_guides() {
        let frame = button();
        assert_eq!(nearest_edge(&frame, Axis::X, 37, 40, 8, Tolerance::DEFAULT), Some(40));
        assert_eq!(nearest_edge(&frame, Axis::X, 41, 40, 8, Tolerance::DEFAULT), Some(40));
        assert_eq!(nearest_edge(&frame, Axis::X, 103, 40, 8, Tolerance::DEFAULT), Some(100));
        assert_eq!(nearest_edge(&frame, Axis::Y, 70, 27, 8, Tolerance::DEFAULT), Some(30));
        assert_eq!(nearest_edge(&frame, Axis::X, 10, 40, 8, Tolerance::DEFAULT), None);
    }

    #[test]
    fn describes_an_element() {
        let frame = button();
        let look = appearance(&frame, Rect::new(40, 30, 60, 20), Tolerance::DEFAULT).unwrap();
        assert_eq!((hex(look.background), look.foreground.map(hex)), ("#2563EB".into(), Some("#FFFFFF".into())));
        assert_eq!(look.insets, Some(Insets { top: 6, right: 12, bottom: 6, left: 12 }));
        let blank = appearance(&Frame::filled(10, 10, GRAY), Rect::new(0, 0, 10, 10), Tolerance::DEFAULT).unwrap();
        assert_eq!((blank.foreground, blank.insets), (None, None));
    }

    #[test]
    fn finds_each_object_in_an_area() {
        let mut frame = Frame::filled(200, 100, WHITE);
        frame.fill(Rect::new(10, 10, 20, 20), BLUE);
        frame.fill(Rect::new(60, 10, 30, 10), GRAY);
        // Two letters 2 px apart: one word.
        frame.fill(Rect::new(120, 40, 6, 10), 0x111111);
        frame.fill(Rect::new(128, 40, 6, 10), 0x111111);
        // A speck.
        frame.fill(Rect::new(180, 80, 1, 1), 0x111111);
        let found = objects(&frame, Rect::new(0, 0, 200, 100), Tolerance::DEFAULT, 3);
        assert_eq!(found, [Rect::new(10, 10, 20, 20), Rect::new(60, 10, 30, 10), Rect::new(120, 40, 14, 10)]);
        assert!(objects(&Frame::filled(10, 10, WHITE), Rect::new(0, 0, 10, 10), Tolerance::DEFAULT, 3).is_empty());
        assert_eq!(aspect_ratio(1920.0, 1080.0).as_deref(), Some("16:9"));
        assert_eq!(aspect_ratio(120.0, 32.0).as_deref(), Some("15:4"));
        assert_eq!(aspect_ratio(173.0, 41.0), None);
        assert_eq!(
            (Tolerance(1).level(), Tolerance::DEFAULT.level(), Tolerance::HIGH.level()),
            ("Low", "Medium", "High")
        );
        assert_eq!(Tolerance::of_level("High"), Some(Tolerance::HIGH));
    }

    /// A frame whose colors drift by `slope` levels a pixel across and down: a gradient, or what glass
    /// blurs behind it.
    fn drifting(width: u32, height: u32, slope: f64) -> Frame {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let level = |offset: f64| ((offset + (x as f64 + y as f64 * 0.5) * slope) % 400.0 - 200.0).abs() as u8;
                rgba.extend_from_slice(&[level(40.0), level(90.0), level(160.0), 255]);
            }
        }
        Frame::new(width, height, rgba).unwrap()
    }

    #[test]
    fn looks_through_what_glass_blurs() {
        // A glass panel at (20, 10), 160 × 80, with a button: inside, the blurred background behind
        // it drifts two levels a pixel, more than the tolerance from end to end.
        let mut frame = Frame::filled(200, 100, WHITE);
        let blur = drifting(160, 80, 2.0);
        for y in 0..80 {
            for x in 0..160 {
                let i = (((y + 10) * 200 + x + 20) * 4) as usize;
                let j = ((y * 160 + x) * 4) as usize;
                frame.rgba[i..i + 4].copy_from_slice(&blur.rgba[j..j + 4]);
            }
        }
        frame.fill(Rect::new(40, 60, 50, 16), 0x6e6e70);
        let d = distance_at(&frame, 120, 40, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!((d.horizontal, d.vertical), (Span { start: 20, end: 180 }, Span { start: 10, end: 90 }));
        let d = distance_at(&frame, 60, 40, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.vertical, Span { start: 10, end: 60 }, "down to the button");
        // An area around the button snaps to it, not to the blur.
        assert_eq!(snap_area(&frame, Rect::new(30, 50, 80, 32), Tolerance::DEFAULT), Some(Rect::new(40, 60, 50, 16)));
        let found = objects(&frame, Rect::new(25, 15, 150, 70), Tolerance::DEFAULT, 3);
        assert_eq!(found, [Rect::new(40, 60, 50, 16)]);
    }

    #[test]
    fn an_area_cutting_into_an_object_still_snaps_to_it() {
        // A drag around the button that cuts into its left edge: the button, as far as the area
        // goes, not the text on it.
        let frame = button();
        assert_eq!(snap_area(&frame, Rect::new(44, 25, 70, 30), Tolerance::DEFAULT), Some(Rect::new(44, 30, 56, 20)));
        let found = objects(&frame, Rect::new(44, 25, 70, 30), Tolerance::DEFAULT, 3);
        assert_eq!(found, [Rect::new(44, 30, 56, 20)]);
    }

    #[test]
    fn soft_shadows_and_gradients_are_not_edges() {
        // A white card on a page that darkens downwards, with a shadow under it that fades into
        // the page over 24 px.
        let page = |y: u32| 0xf6 - y / 8;
        let gray = |level: u32| level << 16 | level << 8 | level;
        let mut frame = Frame::filled(200, 140, WHITE);
        for y in 0..140 {
            frame.fill(Rect::new(0, y, 200, 1), gray(page(y)));
        }
        frame.fill(Rect::new(40, 20, 100, 50), WHITE);
        for i in 0..24u32 {
            let y = 70 + i;
            frame.fill(Rect::new(40, y, 100, 1), gray(page(y) - 30 * (24 - i) / 24));
        }
        let d = distance_at(&frame, 90, 130, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.vertical, Span { start: 70, end: 140 }, "through the shadow and the gradient, up to the card");
        assert_eq!(
            snap_area(&frame, Rect::new(30, 10, 120, 120), Tolerance::DEFAULT),
            Some(Rect::new(40, 20, 100, 50))
        );
    }

    #[test]
    fn sees_faint_anti_aliased_edges() {
        // A card 7 levels off white, its edge spread over two pixels: no step between neighbours is
        // over the tolerance, the two together are.
        let mut frame = Frame::filled(100, 40, WHITE);
        frame.fill(Rect::new(30, 0, 1, 40), 0xfbfbfb);
        frame.fill(Rect::new(31, 0, 69, 40), 0xf8f8f8);
        let d = distance_at(&frame, 10, 20, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.horizontal, Span { start: 0, end: 31 });
        // The pixel between is as close to either side: both runs take it.
        let d = distance_at(&frame, 60, 20, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.horizontal, Span { start: 30, end: 100 });
        // An area across the edge has two backgrounds and nothing drawn on them.
        assert_eq!(snap_area(&frame, Rect::new(5, 5, 90, 30), Tolerance::DEFAULT), None);
        assert_eq!(snap_area(&frame, Rect::new(5, 5, 40, 30), Tolerance::DEFAULT), None);
    }

    #[test]
    fn colors_that_look_the_same_are_not_edges() {
        // A sky blue the screen shows but sRGB can't: its red, clipped at 0, jumps between 0, 3 and
        // 6 from one pixel to the next. A card on it, and a smaller one 6 levels off the first.
        let mut frame = Frame::filled(200, 100, 0x0070b1);
        for y in 0..100 {
            for x in 0..200 {
                let red = [0, 3, 6, 3, 0, 0, 6][((x * 7 + y * 13) ^ (x / 3)) as usize % 7];
                frame.fill(Rect::new(x, y, 1, 1), red << 16 | 0x70b1);
            }
        }
        frame.fill(Rect::new(140, 40, 40, 20), 0xf3f4f6);
        frame.fill(Rect::new(150, 45, 20, 10), 0xf9fafb);
        let d = distance_at(&frame, 60, 50, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!((d.horizontal, d.vertical), (Span { start: 0, end: 140 }, Span { start: 0, end: 100 }));
        assert_eq!(snap_area(&frame, Rect::new(10, 10, 100, 80), Tolerance::DEFAULT), None);
        assert_eq!(snap_area(&frame, Rect::new(130, 30, 60, 40), Tolerance::DEFAULT), Some(Rect::new(140, 40, 40, 20)));
        // The faint card is still an edge: it can be seen.
        let d = distance_at(&frame, 145, 50, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.horizontal, Span { start: 140, end: 150 });
    }

    /// A line's colors, changing from `from` to `to` over `width` pixels from `at` (a smooth step).
    fn blend(from: Rgb, to: Rgb, at: u32, width: u32) -> impl Fn(u32) -> Rgb {
        move |x| {
            let t = (x.saturating_sub(at) as f64 / width as f64).min(1.0);
            let t = t * t * (3.0 - 2.0 * t);
            let mix = |s: u32| {
                let (a, b) = (channel(from, s) as f64, channel(to, s) as f64);
                ((a + (b - a) * t).round() as u32) << s
            };
            mix(16) | mix(8) | mix(0)
        }
    }

    #[test]
    fn looks_through_a_strong_blur_but_not_a_soft_edge() {
        // Dark glass over a cloud and the sky: behind it, the cloud's edge blurred over 40 px,
        // changing by up to 8 levels a pixel. A border around the glass, a button on it.
        let mut frame = Frame::filled(300, 120, 0x0070b1);
        let behind = blend(0x7d7a6e, 0x183f63, 60, 40);
        for x in 20..280 {
            frame.fill(Rect::new(x, 10, 1, 100), behind(x));
        }
        for side in [
            Rect::new(20, 10, 260, 1),
            Rect::new(20, 109, 260, 1),
            Rect::new(20, 10, 1, 100),
            Rect::new(279, 10, 1, 100),
        ] {
            frame.fill(side, 0x5a5a5f);
        }
        frame.fill(Rect::new(200, 70, 50, 20), 0xe6e6eb);
        let d = distance_at(&frame, 80, 40, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!((d.horizontal, d.vertical), (Span { start: 21, end: 279 }, Span { start: 11, end: 109 }));
        assert_eq!(snap_area(&frame, Rect::new(30, 20, 150, 80), Tolerance::DEFAULT), None);
        assert_eq!(
            snap_area(&frame, Rect::new(150, 60, 120, 40), Tolerance::DEFAULT),
            Some(Rect::new(200, 70, 50, 20))
        );
        // The same cloud, sharp, scaled up to the screen: its edge spreads over 8 px, and is one.
        let mut frame = Frame::filled(200, 40, 0x0070b1);
        let cloud = blend(0x0070b1, 0xe6e6dc, 100, 8);
        for x in 0..200 {
            frame.fill(Rect::new(x, 0, 1, 40), cloud(x));
        }
        let d = distance_at(&frame, 40, 20, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert!((100..104).contains(&d.horizontal.end), "{:?}", d.horizontal);
        // A painted cloud's edge, from a wallpaper: changing most over 10 px, then slowly over 15
        // more. It's one too, where it changes most.
        let edge = [
            0x0a7eb5, 0x0a7eb5, 0x077fb6, 0x0d7fb6, 0x0d7fb6, 0x1581b7, 0x1e84b7, 0x2886b6, 0x3289b5, 0x428eb7,
            0x5295b8, 0x649db9, 0x76a4ba, 0x85abbb, 0x90b0bb, 0x96b3ba, 0x9cb7bb, 0xa4bbbb, 0xabc0bd, 0xb3c5bf,
            0xb6c7bd, 0xb9c7bc, 0xbccabd, 0xbfcdbd, 0xc5d0bf, 0xc9d3c0, 0xcbd4bf, 0xccd5be, 0xced7bd, 0xced8bd,
        ];
        let mut frame = Frame::filled(120, 40, 0xced8bd);
        frame.fill(Rect::new(0, 0, 50, 40), 0x0a7eb5);
        for (i, &rgb) in edge.iter().enumerate() {
            frame.fill(Rect::new(50 + i as u32, 0, 1, 40), rgb);
        }
        let d = distance_at(&frame, 20, 20, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert!((55..61).contains(&d.horizontal.end), "{:?}", d.horizontal);
    }

    /// A sky in the colors a freeze has, `w` × `h`: its red clipped near 0 and every channel grainy.
    fn grainy_sky(w: u32, h: u32, seed: u32) -> Frame {
        let mut seed = seed;
        let mut grain = move |most: u32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 16) % (most + 1)
        };
        let mut frame = Frame::filled(w, h, 0);
        for y in 0..h {
            for x in 0..w {
                frame.fill(Rect::new(x, y, 1, 1), grain(7) << 16 | (112 + grain(3)) << 8 | (174 + grain(5)));
            }
        }
        frame
    }

    #[test]
    fn looks_past_grain_but_not_faint_outlines() {
        // A grainy sky; on it a faint card (8 levels lighter) and a hairline as faint, and a dot.
        let mut frame = grainy_sky(240, 160, 1);
        frame.fill(Rect::new(140, 30, 80, 60), 0x0e7cba);
        frame.fill(Rect::new(10, 120, 220, 1), 0x0e7cba);
        frame.fill(Rect::new(60, 60, 2, 2), 0xffffff);
        // From the open sky: to the card, the hairline and the frame's edges, past every speck.
        let d = distance_at(&frame, 70, 40, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!((d.horizontal, d.vertical), (Span { start: 0, end: 140 }, Span { start: 0, end: 120 }));
        // The dot stops a line through it.
        let d = distance_at(&frame, 30, 61, Tolerance::DEFAULT, &Stops::default()).unwrap();
        assert_eq!(d.horizontal, Span { start: 0, end: 60 });
        assert_eq!(snap_area(&frame, Rect::new(20, 10, 100, 100), Tolerance::DEFAULT), Some(Rect::new(60, 60, 2, 2)));
        assert_eq!(
            snap_area(&frame, Rect::new(100, 10, 135, 100), Tolerance::DEFAULT),
            Some(Rect::new(140, 30, 80, 60))
        );
    }

    /// `rect` filled with `rgb` on `frame`, its corners rounded by `radius` and anti-aliased.
    fn rounded(frame: &mut Frame, rect: Rect, radius: f64, rgb: Rgb) {
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                // How far the pixel's center is outside the corner's circle, if it's in a corner.
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let near = |v: f64, lo: u32, hi: u32| v.clamp(lo as f64 + radius, hi as f64 - radius);
                let (dx, dy) = (cx - near(cx, rect.x, rect.right()), cy - near(cy, rect.y, rect.bottom()));
                let cover = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                let under = frame.rgb(x, y);
                let mix = |s: u32| {
                    let (a, b) = (channel(under, s) as f64, channel(rgb, s) as f64);
                    ((a + (b - a) * cover).round() as u32) << s
                };
                frame.fill(Rect::new(x, y, 1, 1), mix(16) | mix(8) | mix(0));
            }
        }
    }

    #[test]
    fn faint_corners_and_hairline_ends_are_edges() {
        // A selected item in a sidebar, 6 levels darker, its corners rounded; a hairline as faint, 2 px.
        let mut frame = Frame::filled(240, 140, 0xf9fafb);
        rounded(&mut frame, Rect::new(20, 20, 200, 60), 16.0, 0xf3f4f6);
        frame.fill(Rect::new(40, 110, 160, 2), 0xf3f4f6);
        // Across its rows, even where its corners run nearly along them (but for the three at the top
        // and bottom, where the corners change too little from a pixel to the next).
        for y in 24..76 {
            let d = distance_at(&frame, 120, y, Tolerance::DEFAULT, &Stops::default()).unwrap();
            assert!(d.horizontal.start >= 20 && d.horizontal.end <= 220, "row {y}: {:?}", d.horizontal);
        }
        // The hairline's ends are edges too, so an area around it finds all of it.
        assert_eq!(
            snap_area(&frame, Rect::new(30, 100, 180, 20), Tolerance::DEFAULT),
            Some(Rect::new(40, 110, 160, 2))
        );
    }

    #[test]
    fn looks_past_dense_grain() {
        // Grain on every pixel, lighter or darker by up to 12 levels, heavier than a wallpaper's: from
        // nearly anywhere, measuring goes past it for 50 px or more (0.7.1 stopped every few pixels).
        let mut seed = 7u32;
        let mut grain = move |most: u32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 16) % (most + 1)
        };
        let mut frame = Frame::filled(300, 200, 0);
        for y in 0..200 {
            for x in 0..300 {
                let g = grain(12);
                let rgb = (0x56 + g + grain(1)) << 16 | (0x85 + g + grain(1)) << 8 | (0xd5 + g + grain(1));
                frame.fill(Rect::new(x, y, 1, 1), rgb);
            }
        }
        let mut spans = Vec::new();
        for y in [20, 60, 100, 140, 180] {
            for x in [30, 90, 150, 210, 270] {
                let d = distance_at(&frame, x, y, Tolerance::DEFAULT, &Stops::default()).unwrap();
                spans.extend([d.horizontal.len(), d.vertical.len()]);
            }
        }
        assert!(spans.iter().filter(|&&n| n >= 50).count() >= 40, "{spans:?}");
    }

    #[test]
    fn snapping_finds_the_edges_measuring_does() {
        // Grain, faint and strong shapes, sharp and soft, slanted: contours followed for a block of
        // lines at a time, across blocks and threads, find what following each from its change does.
        let mut frame = grainy_sky(700, 420, 2);
        frame.fill(Rect::new(40, 30, 300, 200), 0x0b79b8);
        frame.fill(Rect::new(400, 60, 1, 300), 0x0b79b8);
        for i in 0..200 {
            frame.fill(Rect::new(450 + i, 100 + i, 30, 1), 0xd0d8e0);
            frame.fill(Rect::new(100 + i / 3, 250 + i / 2, 12, 1), 0x2080c0);
        }
        let cloud = blend(0x0070b1, 0xe6e6dc, 560, 8);
        for x in 520..600 {
            frame.fill(Rect::new(x, 20, 1, 60), cloud(x));
        }
        let threshold = Tolerance::DEFAULT.threshold();
        for rect in [Rect::new(0, 0, 700, 420), Rect::new(37, 61, 300, 150), Rect::new(600, 300, 100, 120)] {
            let content = content_map(&frame, rect, threshold);
            assert_eq!(content.len(), (rect.width * rect.height) as usize);
            let mut out = vec![0u8; (rect.width * rect.height) as usize];
            boundaries(&frame, rect.y..rect.bottom(), rect.x..rect.right(), threshold, &mut out);
            for rows in [true, false] {
                let line_of = |l| if rows { Line::row(&frame, l) } else { Line::column(&frame, l) };
                let (lines, positions) = match rows {
                    true => (rect.y..rect.bottom(), rect.x..rect.right()),
                    false => (rect.x..rect.right(), rect.y..rect.bottom()),
                };
                let n = positions.len();
                for (i, l) in lines.clone().enumerate() {
                    let line = line_of(l);
                    let edge = |from: u32, to: u32| is_edge(&line, from, to, threshold);
                    let differ = |p: u32| difference(line.rgb(p), line.rgb(p + 1)) > threshold;
                    for (k, p) in (1..n).zip(positions.start + 1..) {
                        let traced = edge(p - 1, p)
                            || (k >= 2 && !differ(p - 2) && edge(p - 2, p))
                            || (k + 1 < n && !differ(p) && edge(p - 1, p + 1));
                        let found = match rows {
                            true => out[i * n + k] & ACROSS != 0,
                            false => out[k * lines.len() + i] & DOWN != 0,
                        };
                        assert_eq!(found, traced, "line {l}, pixel {p} of {rect:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn crops_frames() {
        let frame = button();
        let crop = frame.crop(Rect::new(38, 28, 4, 4));
        assert_eq!((crop.width, crop.height), (4, 4));
        assert_eq!((crop.rgb(0, 0), crop.rgb(3, 3)), (WHITE, BLUE));
        assert!(Frame::new(2, 2, vec![0; 15]).is_none());
        assert_eq!(difference(0x102030, 0x0f2f30), 15);
    }
}
