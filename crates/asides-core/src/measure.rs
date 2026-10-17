//! Measuring what's on screen, the way PixelSnap does: from a point, the distance to the nearest
//! edge in each direction; inside a dragged area, the bounds of the object it surrounds; near a
//! guide, the edge it should snap to. Everything works on a frozen frame (a screenshot) in pixels,
//! origin top-left. An edge is where the color changes sharply: a pixel differs from the one before
//! it, or from the one two before (an edge anti-aliasing spread over two pixels), by more than the
//! tolerance. Slow changes aren't edges, however far they go: gradients, soft shadows, and what
//! glass and frosted materials blur behind them (one or two levels a pixel, where an edge drawn on
//! them jumps by tens). Raising the tolerance also looks through faint borders and hard shadows.

use std::collections::HashMap;

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
    let channel = |c: Rgb, shift: u32| ((c >> shift) & 0xff) as i32;
    [16, 8, 0].iter().map(|&s| (channel(a, s) - channel(b, s)).unsigned_abs()).max().unwrap_or(0) as u8
}

/// `#RRGGBB`.
pub fn hex(rgb: Rgb) -> String {
    format!("#{:06X}", rgb & 0xff_ffff)
}

/// Whether the color changes sharply at `at`, coming from `before`: by more than `threshold` from
/// the pixel just before, or from the one two before (`two_before`, when it's on the same run), for
/// an edge that anti-aliasing spread over two pixels.
fn sharp(at: Rgb, before: Rgb, two_before: Option<Rgb>, threshold: u8) -> bool {
    difference(at, before) > threshold || two_before.is_some_and(|p| difference(at, p) > threshold)
}

/// The run through `at` on a line of `len` pixels (`pixel(i)`), up to the first sharp change on
/// either side.
fn run(len: u32, at: u32, threshold: u8, pixel: impl Fn(u32) -> Rgb) -> Span {
    let mut end = at + 1;
    while end < len && !sharp(pixel(end), pixel(end - 1), (end >= at + 2).then(|| pixel(end - 2)), threshold) {
        end += 1;
    }
    let mut start = at;
    while start > 0 && !sharp(pixel(start - 1), pixel(start), (start < at).then(|| pixel(start + 1)), threshold) {
        start -= 1;
    }
    Span { start, end }
}

/// The distances from `(x, y)` to the nearest edge left, right, up and down, stopping at guides.
pub fn distance_at(frame: &Frame, x: u32, y: u32, tolerance: Tolerance, stops: &Stops) -> Option<Distance> {
    if !frame.contains(x, y) {
        return None;
    }
    let threshold = tolerance.threshold();
    let across = run(frame.width, x, threshold, |i| frame.rgb(i, y));
    let down = run(frame.height, y, threshold, |i| frame.rgb(x, i));
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

/// Which pixels of `rect` (row by row) are drawn on its background: those the background on its
/// outline can't reach without crossing a sharp change, stepping across and down. What the
/// background does on the way doesn't matter (a gradient, a soft shadow, what glass blurs behind
/// it); what's drawn on it has edges. The outline's background is its longest stretch without a
/// sharp change, and the stretches colored like it: what crosses the outline (a button the area
/// cuts into) is drawn on it.
///
/// Between two neighbours there's an edge when they differ by more than `threshold`, or when one
/// of them and the pixel beyond the other do while that pair is alike: an edge that anti-aliasing
/// spread over two pixels, whichever way it's crossed. The edges are found a row at a time, then
/// the background filled a stretch of a row at a time (a scanline fill).
fn content_map(frame: &Frame, rect: Rect, threshold: u8) -> Vec<bool> {
    let (w, h) = (rect.width as usize, rect.height as usize);
    let stride = frame.width as usize * 4;
    let row = |y: usize| {
        let start = (rect.y as usize + y) * stride + rect.x as usize * 4;
        &frame.rgba[start..start + w * 4]
    };
    // Whether each pixel of `a` differs from the one at the same place in `b` (byte by byte first,
    // which vectorizes).
    let mut bytes = vec![0u8; w * 4];
    let mut differ = |a: &[u8], b: &[u8], out: &mut [bool]| {
        let n = a.len().min(b.len());
        for ((d, p), q) in bytes[..n].iter_mut().zip(&a[..n]).zip(&b[..n]) {
            *d = p.abs_diff(*q);
        }
        for (px, d) in bytes[..n].as_chunks::<4>().0.iter().zip(out.iter_mut()) {
            *d = px[0].max(px[1]).max(px[2]) > threshold;
        }
    };
    // Across: `edge_across[y * w + x]`, between (x - 1, y) and (x, y).
    let mut edge_across = vec![false; w * h];
    let (mut one, mut two) = (vec![false; w], vec![false; w]);
    for y in 0..h {
        let r = row(y);
        // one[k]: pixels k and k + 1 differ; two[k]: k and k + 2.
        differ(&r[..w.saturating_sub(1) * 4], &r[4.min(r.len())..], &mut one);
        differ(&r[..w.saturating_sub(2) * 4], &r[8.min(r.len())..], &mut two);
        for x in 1..w {
            edge_across[y * w + x] =
                one[x - 1] || (x >= 2 && two[x - 2] && !one[x - 2]) || (x + 1 < w && two[x - 1] && !one[x]);
        }
    }
    // Down: rows that differ by one and by two, then `edge_down[y * w + x]`, between (x, y - 1)
    // and (x, y).
    let mut down_one = vec![false; w * h];
    let mut down_two = vec![false; w * h];
    for y in 1..h {
        differ(row(y - 1), row(y), &mut down_one[y * w..(y + 1) * w]);
        if y >= 2 {
            differ(row(y - 2), row(y), &mut down_two[y * w..(y + 1) * w]);
        }
    }
    let mut edge_down = vec![false; w * h];
    for y in 1..h {
        for x in 0..w {
            let i = y * w + x;
            edge_down[i] = down_one[i]
                || (y >= 2 && down_two[i] && !down_one[i - w])
                || (y + 1 < h && down_two[i + w] && !down_one[i + w]);
        }
    }
    drop((down_one, down_two));
    let mut reached = vec![false; w * h];
    let pixel = |x: usize, y: usize| {
        let i = (rect.y as usize + y) * stride + (rect.x as usize + x) * 4;
        [frame.rgba[i], frame.rgba[i + 1], frame.rgba[i + 2]]
    };
    let mut stack = outline_background(w, h, threshold, pixel, |a, b| {
        // Neighbours on the outline: across or down from one another.
        let (first, second) = if (a.1, a.0) < (b.1, b.0) { (a, b) } else { (b, a) };
        if first.1 == second.1 { edge_across[second.1 * w + second.0] } else { edge_down[second.1 * w + second.0] }
    });
    while let Some((x, y)) = stack.pop() {
        let row = y * w;
        if reached[row + x] {
            continue;
        }
        let (mut left, mut right) = (x, x);
        while left > 0 && !reached[row + left - 1] && !edge_across[row + left] {
            left -= 1;
        }
        while right + 1 < w && !reached[row + right + 1] && !edge_across[row + right + 1] {
            right += 1;
        }
        reached[row + left..=row + right].fill(true);
        // Seeds in the rows above and below: one per stretch that this one opens onto.
        for (other, boundary) in [(y.checked_sub(1), y), ((y + 1 < h).then_some(y + 1), y + 1)] {
            let Some(other) = other else { continue };
            let mut open_before = false;
            for cx in left..=right {
                let open = !reached[other * w + cx] && !edge_down[boundary * w + cx];
                if open && (!open_before || edge_across[other * w + cx]) {
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
    let content = content_map(frame, rect, tolerance.threshold());
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
    let (at, limit) = match axis {
        Axis::X => (x, frame.width),
        Axis::Y => (y, frame.height),
    };
    let pixel = |i: u32| match axis {
        Axis::X => frame.rgb(i, y),
        Axis::Y => frame.rgb(x, i),
    };
    let is_edge = |b: u32| b > 0 && b < limit && difference(pixel(b - 1), pixel(b)) > threshold;
    // The boundaries `d` pixels away on either side of the pixel at `at`, closest first.
    (0..=radius).find_map(|d| [at.checked_sub(d), at.checked_add(d + 1)].into_iter().flatten().find(|&b| is_edge(b)))
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
    let content = snap_area(frame, rect, tolerance);
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
    fn crops_frames() {
        let frame = button();
        let crop = frame.crop(Rect::new(38, 28, 4, 4));
        assert_eq!((crop.width, crop.height), (4, 4));
        assert_eq!((crop.rgb(0, 0), crop.rgb(3, 3)), (WHITE, BLUE));
        assert!(Frame::new(2, 2, vec![0; 15]).is_none());
        assert_eq!(difference(0x102030, 0x0f2f30), 15);
    }
}
