//! Measuring a screenshot file, PixelSnap-style: `asides measure` and the MCP tool `measure_image`.
//! Agents use it to check spacing and sizes in the user's screenshots, or in their own.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};
use asides_core::measure::{self, Frame, Rect, Stops, Tolerance};

/// What to measure in an image, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// The distances from this point to the nearest edges.
    Point(u32, u32),
    /// The bounds of the object inside this area.
    Area(Rect),
}

/// Decodes a PNG into straight RGBA.
pub fn load(path: &Path) -> Result<Frame> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().with_context(|| format!("{} is not a PNG", path.display()))?;
    let mut buf = vec![0; reader.output_buffer_size().context("image too large")?];
    let info = reader.next_frame(&mut buf).with_context(|| format!("could not decode {}", path.display()))?;
    let pixels = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("unexpected indexed colors in {}", path.display()),
    };
    Frame::new(info.width, info.height, rgba).context("unexpected pixel data")
}

/// The measurement as text: pixels, plus points when `scale` (pixels per point) isn't 1.
pub fn describe(frame: &Frame, name: &str, target: Target, tolerance: Tolerance, scale: f64) -> Result<String> {
    let size = format!("{} × {} px", frame.width, frame.height);
    let len = |px: u32| -> String {
        if scale == 1.0 {
            format!("{px} px")
        } else {
            let pt = px as f64 / scale;
            let pt = if pt.fract() == 0.0 { format!("{pt}") } else { format!("{pt:.1}") };
            format!("{px} px ({pt} pt at @{scale}x)")
        }
    };
    let tolerance_note = format!("Tolerance {}% (edges are where a color differs by more than that).", tolerance.0);
    match target {
        Target::Point(x, y) => {
            let Some(d) = measure::distance_at(frame, x, y, tolerance, &Stops::default()) else {
                bail!("({x}, {y}) is outside {name} ({size})");
            };
            let side = |open: bool, what: &str, at: String| if open { format!("the image's {what} edge") } else { at };
            let (open_left, open_right) = d.horizontal.open(frame.width);
            let (open_top, open_bottom) = d.vertical.open(frame.height);
            Ok(format!(
                "Distances from ({x}, {y}) in {name} ({size}), where the color is {}:\n\
                 - across: {}, from {} to {}\n\
                 - down: {}, from {} to {}\n\
                 {tolerance_note}",
                measure::hex(frame.rgb(x, y)),
                len(d.horizontal.len()),
                side(open_left, "left", format!("an edge at x {}", d.horizontal.start)),
                side(open_right, "right", format!("an edge at x {}", d.horizontal.end)),
                len(d.vertical.len()),
                side(open_top, "top", format!("an edge at y {}", d.vertical.start)),
                side(open_bottom, "bottom", format!("an edge at y {}", d.vertical.end)),
            ))
        }
        Target::Area(rect) => {
            let area = rect.clamp_to(frame.width, frame.height);
            if area.is_empty() {
                bail!("the area is outside {name} ({size})");
            }
            let mut text =
                format!("Area ({}, {}) {} × {} px in {name} ({size}):\n", area.x, area.y, area.width, area.height);
            let Some(look) = measure::appearance(frame, area, tolerance) else { bail!("the area is empty") };
            match look.content {
                Some(c) => {
                    text.push_str(&format!(
                        "- the object inside: ({}, {}), {} wide × {} high\n",
                        c.x,
                        c.y,
                        len(c.width),
                        len(c.height)
                    ));
                    if let Some(i) = look.insets {
                        text.push_str(&format!(
                            "- space around it in the area: top {}, right {}, bottom {}, left {}\n",
                            len(i.top),
                            len(i.right),
                            len(i.bottom),
                            len(i.left)
                        ));
                    }
                }
                None => text.push_str("- nothing inside: the whole area is one color\n"),
            }
            text.push_str(&format!("- background {}", measure::hex(look.background)));
            if let Some(fg) = look.foreground {
                text.push_str(&format!(", most common other color {}", measure::hex(fg)));
            }
            text.push_str(&format!("\n{tolerance_note}"));
            Ok(text)
        }
    }
}

/// `412,230` → a point; `380,200,320,48` → an area.
pub fn parse_target(s: &str) -> Result<Target> {
    let numbers: Vec<u32> = s
        .split(',')
        .map(|n| n.trim().parse::<f64>().map(|v| v.max(0.0).round() as u32))
        .collect::<Result<_, _>>()
        .with_context(|| format!("\"{s}\": expected X,Y or X,Y,WIDTH,HEIGHT in pixels"))?;
    match numbers.as_slice() {
        [x, y] => Ok(Target::Point(*x, *y)),
        [x, y, w, h] if *w > 0 && *h > 0 => Ok(Target::Area(Rect::new(*x, *y, *w, *h))),
        _ => bail!("\"{s}\": expected X,Y or X,Y,WIDTH,HEIGHT in pixels"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, frame: &Frame) {
        let file = File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, frame.width, frame.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&frame.rgba).unwrap();
    }

    #[test]
    fn measures_a_png() {
        let tmp = tempfile::tempdir().unwrap();
        let mut frame = Frame::filled(200, 100, 0xffffff);
        frame.fill(Rect::new(40, 30, 60, 20), 0x2563eb);
        let path = tmp.path().join("shot.png");
        write_png(&path, &frame);
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, frame);

        let text = describe(&loaded, "shot.png", parse_target("10,40").unwrap(), Tolerance::DEFAULT, 2.0).unwrap();
        assert!(
            text.contains("- across: 40 px (20 pt at @2x), from the image's left edge to an edge at x 40"),
            "{text}"
        );
        assert!(text.contains("where the color is #FFFFFF"), "{text}");
        let text =
            describe(&loaded, "shot.png", parse_target("30,20,100,50").unwrap(), Tolerance::DEFAULT, 1.0).unwrap();
        assert!(text.contains("- the object inside: (40, 30), 60 px wide × 20 px high"), "{text}");
        assert!(text.contains("top 10 px, right 30 px, bottom 20 px, left 10 px"), "{text}");
        assert!(describe(&loaded, "shot.png", Target::Point(500, 5), Tolerance::DEFAULT, 1.0).is_err());
        assert!(parse_target("1,2,3").is_err() && parse_target("a,b").is_err());
        assert!(load(&tmp.path().join("missing.png")).is_err());
    }
}
