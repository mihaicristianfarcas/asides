//! The clipboard on Linux, through X11 (Xwayland on a Wayland desktop, which shares it with
//! everything): text, and a screenshot as other apps paste it. What's on the X11 clipboard is served
//! by whoever put it there, so the handle lives as long as the app. What the user had there is kept
//! as text while an aside's screenshot is put on it, to put back.

use std::borrow::Cow;
use std::sync::{Mutex, OnceLock};

/// The clipboard, for as long as the app runs (`None` without X11).
fn clipboard() -> Option<&'static Mutex<arboard::Clipboard>> {
    static CLIPBOARD: OnceLock<Option<Mutex<arboard::Clipboard>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| arboard::Clipboard::new().ok().map(Mutex::new)).as_ref()
}

/// What the clipboard held: its text, if it had any.
pub struct Snapshot(Option<String>);

/// Keeps what the clipboard holds, to put back with [`restore`].
pub fn snapshot() -> Snapshot {
    Snapshot(clipboard().and_then(|c| c.lock().ok()?.get_text().ok()))
}

/// Puts back what [`snapshot`] kept.
pub fn restore(snapshot: &Snapshot) {
    let Some(Ok(mut clipboard)) = clipboard().map(Mutex::lock) else { return };
    let _ = match &snapshot.0 {
        Some(text) => clipboard.set_text(text.clone()),
        None => clipboard.clear(),
    };
}

/// Puts text on the clipboard.
pub fn put_text(text: &str) {
    if let Some(Ok(mut clipboard)) = clipboard().map(Mutex::lock) {
        let _ = clipboard.set_text(text.to_string());
    }
}

/// Puts a PNG on the clipboard, as a screenshot is copied.
pub fn put_png(bytes: &[u8]) {
    let Some((width, height, rgba)) = decode(bytes) else { return };
    if let Some(Ok(mut clipboard)) = clipboard().map(Mutex::lock) {
        let image = arboard::ImageData { width, height, bytes: Cow::Owned(rgba) };
        let _ = clipboard.set_image(image);
    }
}

/// A PNG's pixels as RGBA.
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.as_chunks::<2>().0.iter().flat_map(|&[v, a]| [v, v, v, a]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((info.width as usize, info.height as usize, rgba))
}
