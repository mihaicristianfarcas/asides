//! Measuring what's on screen, the way PixelSnap does: from a point, the distance to the nearest
//! edge in each direction; inside a dragged area, the bounds of the object it surrounds; near a
//! guide, the edge it should snap to. Everything works on a frozen frame (a screenshot) in pixels,
//! origin top-left. An edge is where a pixel differs from where the scan started by more than the
//! tolerance, so raising the tolerance looks through shadows, gradients and anti-aliasing.

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

/// How different two colors must be to count as an edge, in percent of the full channel range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tolerance(pub u8);

impl Tolerance {
    /// Low enough to see a light gray card on white (#F3F4F6 differs by 12 of 255).
    pub const DEFAULT: Tolerance = Tolerance(2);
    /// High enough to look through drop shadows and soft gradients (PixelSnap's tolerance mode).
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

/// The distances from `(x, y)` to the nearest edge left, right, up and down, stopping at guides.
pub fn distance_at(frame: &Frame, x: u32, y: u32, tolerance: Tolerance, stops: &Stops) -> Option<Distance> {
    if !frame.contains(x, y) {
        return None;
    }
    let threshold = tolerance.threshold();
    let origin = frame.rgb(x, y);
    let same = |px: u32, py: u32| difference(frame.rgb(px, py), origin) <= threshold;

    let mut left = x;
    while left > 0 && same(left - 1, y) {
        left -= 1;
    }
    let mut right = x + 1;
    while right < frame.width && same(right, y) {
        right += 1;
    }
    let mut top = y;
    while top > 0 && same(x, top - 1) {
        top -= 1;
    }
    let mut bottom = y + 1;
    while bottom < frame.height && same(x, bottom) {
        bottom += 1;
    }
    let (left, right) = clip_to_stops(left, right, x, &stops.x);
    let (top, bottom) = clip_to_stops(top, bottom, y, &stops.y);
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

/// The bounds of whatever `rect` surrounds: the smallest rectangle holding every pixel that
/// differs from the background (see [`background`]). `None` when the area is all background.
pub fn snap_area(frame: &Frame, rect: Rect, tolerance: Tolerance) -> Option<Rect> {
    let rect = rect.clamp_to(frame.width, frame.height);
    let bg = background(frame, rect)?;
    let threshold = tolerance.threshold();
    let content = |x: u32, y: u32| difference(frame.rgb(x, y), bg) > threshold;
    let row_has = |y: u32| (rect.x..rect.right()).any(|x| content(x, y));
    let top = (rect.y..rect.bottom()).find(|&y| row_has(y))?;
    let bottom = (top..rect.bottom()).rev().find(|&y| row_has(y))?;
    let column_has = |x: u32| (top..=bottom).any(|y| content(x, y));
    let left = (rect.x..rect.right()).find(|&x| column_has(x))?;
    let right = (left..rect.right()).rev().find(|&x| column_has(x))?;
    Some(Rect::from_corners((left, top), (right, bottom)))
}

/// The separate objects inside `rect`: groups of pixels that differ from the background, each as its
/// bounds, largest first. Groups within `gap` pixels of each other are one object (the letters of a
/// word, the parts of an icon), and specks smaller than `gap` on both sides are left out.
pub fn objects(frame: &Frame, rect: Rect, tolerance: Tolerance, gap: u32) -> Vec<Rect> {
    let rect = rect.clamp_to(frame.width, frame.height);
    let Some(bg) = background(frame, rect) else { return Vec::new() };
    let threshold = tolerance.threshold();
    let (w, h) = (rect.width as usize, rect.height as usize);
    let mut content: Vec<bool> = Vec::with_capacity(w * h);
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            content.push(difference(frame.rgb(x, y), bg) > threshold);
        }
    }
    // Flood-fill each group of touching content pixels, keeping its bounds.
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
            let neighbours = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then(|| i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then(|| i + w),
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
    let threshold = tolerance.threshold();
    let mut counts: HashMap<Rgb, u32> = HashMap::new();
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            let rgb = frame.rgb(x, y);
            if difference(rgb, bg) > threshold {
                *counts.entry(rgb).or_default() += 1;
            }
        }
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
