//! The general pasteboard, while an aside's screenshots are pasted into a session (Ctrl+V): a
//! screenshot put on it, and what the user had there kept, to put back. And an aside copied, for
//! one paste anywhere.

use asides_core::rich::{Block, Rich};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{
    NSAttributedStringAppKitDocumentFormats, NSAttributedStringAttachmentConveniences, NSFont, NSFontAttributeName,
    NSFontWeightRegular, NSPasteboard, NSPasteboardItem, NSPasteboardTypeHTML, NSPasteboardTypePNG,
    NSPasteboardTypeRTFD, NSPasteboardTypeString, NSPasteboardWriting, NSTextAttachment,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSData, NSDictionary, NSFileWrapper, NSMutableAttributedString,
    NSRange, NSString,
};

/// What the pasteboard holds: each item's data, of each of its types.
pub struct Snapshot(Vec<Vec<(String, Vec<u8>)>>);

/// Keeps what the pasteboard holds, to put back with [`restore`].
pub fn snapshot() -> Snapshot {
    let pasteboard = NSPasteboard::generalPasteboard();
    let items = pasteboard.pasteboardItems().map(|items| items.to_vec()).unwrap_or_default();
    Snapshot(
        items
            .iter()
            .map(|item| {
                item.types().iter().filter_map(|t| Some((t.to_string(), item.dataForType(&t)?.to_vec()))).collect()
            })
            .collect(),
    )
}

/// Puts back what [`snapshot`] kept.
pub fn restore(snapshot: &Snapshot) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
        .0
        .iter()
        .map(|types| {
            let item = NSPasteboardItem::new();
            for (t, data) in types {
                item.setData_forType(&NSData::with_bytes(data), &NSString::from_str(t));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if !items.is_empty() {
        pasteboard.writeObjects(&NSArray::from_retained_slice(&items));
    }
}

/// Puts text on the pasteboard.
pub fn put_text(text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setString_forType(&NSString::from_str(text), unsafe { objc2_app_kit::NSPasteboardTypeString });
}

/// Puts a PNG on the pasteboard, as a screenshot is copied.
pub fn put_png(bytes: &[u8]) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setData_forType(Some(&NSData::with_bytes(bytes)), unsafe { NSPasteboardTypePNG });
}

/// Puts an aside on the pasteboard, as each app pastes it. Its first item holds the Markdown with the
/// screenshots' paths (a terminal's paste, for an agent), and HTML and RTFD with the screenshots in
/// them (a web page's paste, a mail's, a note's); then one item per screenshot, which a chat's paste
/// attaches. Measured in WebKit: items with file URLs would hide the text from a page, and a page's
/// own paste takes only the first item, so nothing comes twice.
pub fn put_aside(rich: &Rich) {
    write_aside(&NSPasteboard::generalPasteboard(), rich);
}

fn write_aside(pasteboard: &NSPasteboard, rich: &Rich) {
    let first = NSPasteboardItem::new();
    first.setString_forType(&NSString::from_str(&rich.text), unsafe { NSPasteboardTypeString });
    first.setString_forType(&NSString::from_str(&rich.html()), unsafe { NSPasteboardTypeHTML });
    if let Some(rtfd) = rtfd(rich) {
        first.setData_forType(&rtfd, unsafe { NSPasteboardTypeRTFD });
    }
    let mut items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = vec![ProtocolObject::from_retained(first)];
    for shot in &rich.images {
        let item = NSPasteboardItem::new();
        // NSPasteboardTypePNG's type, and JPEG's.
        let kind = NSString::from_str(if shot.mime == "image/png" { "public.png" } else { "public.jpeg" });
        item.setData_forType(&NSData::with_bytes(&shot.bytes), &kind);
        items.push(ProtocolObject::from_retained(item));
    }
    pasteboard.clearContents();
    pasteboard.writeObjects(&NSArray::from_retained_slice(&items));
}

/// An aside's blocks as RTFD: its paragraphs in the system font, headings bold, code monospaced,
/// and its screenshots as attachments where they were made.
fn rtfd(rich: &Rich) -> Option<Retained<NSData>> {
    let text = NSMutableAttributedString::new();
    let regular = NSFont::systemFontOfSize(13.0);
    let bold = NSFont::boldSystemFontOfSize(13.0);
    let code = NSFont::monospacedSystemFontOfSize_weight(12.0, unsafe { NSFontWeightRegular });
    let append = |string: &str, font: &NSFont| {
        let keys: [&NSAttributedStringKey; 1] = unsafe { [NSFontAttributeName] };
        let values: [&AnyObject; 1] = [font];
        let attributes = NSDictionary::from_slices(&keys, &values);
        let run = unsafe {
            NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &NSString::from_str(string),
                Some(&attributes),
            )
        };
        text.appendAttributedString(&run);
    };
    for (i, block) in rich.blocks.iter().enumerate() {
        if i > 0 {
            append("\n\n", &regular);
        }
        match block {
            Block::Text(lines) => {
                for (j, line) in lines.iter().enumerate() {
                    if j > 0 {
                        append("\n", &regular);
                    }
                    for span in line {
                        append(
                            &span.text,
                            if span.code {
                                &code
                            } else if span.bold {
                                &bold
                            } else {
                                &regular
                            },
                        );
                    }
                }
            }
            Block::Image(n) => {
                let shot = &rich.images[*n];
                let wrapper = NSFileWrapper::initRegularFileWithContents(
                    NSFileWrapper::alloc(),
                    &NSData::with_bytes(&shot.bytes),
                );
                wrapper.setPreferredFilename(Some(&NSString::from_str(&format!(
                    "Screenshot {}.{}",
                    n + 1,
                    shot.extension
                ))));
                let attachment = NSTextAttachment::initWithFileWrapper(NSTextAttachment::alloc(), Some(&wrapper));
                text.appendAttributedString(&NSAttributedString::attributedStringWithAttachment(&attachment));
            }
        }
    }
    let range = NSRange::new(0, text.length());
    unsafe { text.RTFDFromRange_documentAttributes(range, &NSDictionary::new()) }
}

#[cfg(test)]
mod tests {
    use asides_core::markdown::Part;
    use objc2_app_kit::NSAttachmentAttributeName;

    use super::*;

    /// 1×1 PNGs, red and blue.
    const RED: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90\x77\x53\xde\x00\x00\x00\x0cIDAT\x78\xda\x63\xf8\xaf\x1b\x0a\x00\x03\xb0\x01\x82\xe8\x9d\xe3\x0a\x00\x00\x00\x00IEND\xae\x42\x60\x82";
    const BLUE: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90\x77\x53\xde\x00\x00\x00\x0cIDAT\x78\xda\x63\xb0\x6e\xfa\x06\x00\x02\xaf\x01\xb4\x89\xb2\x7e\xd5\x00\x00\x00\x00IEND\xae\x42\x60\x82";

    #[test]
    fn copies_an_aside_for_every_app() {
        let parts = vec![
            Part::Text("**In:** Safari (\"Shop\")\n\nThe Save button [1] sits low.".into()),
            Part::Image("/h/a/images/1.png".into()),
            Part::Text("### 1. button `#save`\n**Feedback:** Line it up.".into()),
            Part::Image("/h/a/images/2.png".into()),
        ];
        let rich = Rich::from_parts(&parts, |path| Some(if path.ends_with("1.png") { RED } else { BLUE }.to_vec()));
        // A pasteboard of its own; ASIDES_COPY_DUMP=<name> leaves it under that name for a look.
        let name = std::env::var("ASIDES_COPY_DUMP").unwrap_or_else(|_| format!("asides-test-{}", std::process::id()));
        let pasteboard = NSPasteboard::pasteboardWithName(&NSString::from_str(&name));
        write_aside(&pasteboard, &rich);

        let items = pasteboard.pasteboardItems().map(|items| items.to_vec()).unwrap_or_default();
        assert_eq!(items.len(), 3);
        let types = |item: &NSPasteboardItem| item.types().iter().map(|t| t.to_string()).collect::<Vec<_>>();
        assert_eq!(types(&items[0]), ["public.utf8-plain-text", "public.html", "com.apple.flat-rtfd"]);
        let string = |kind: &NSString| items[0].stringForType(kind).map(|s| s.to_string());
        assert_eq!(string(unsafe { NSPasteboardTypeString }).as_deref(), Some(rich.text.as_str()));
        assert_eq!(string(unsafe { NSPasteboardTypeHTML }), Some(rich.html()));
        for (item, png) in items[1..].iter().zip([RED, BLUE]) {
            assert_eq!(types(item), ["public.png"]);
            assert_eq!(item.dataForType(unsafe { NSPasteboardTypePNG }).map(|d| d.to_vec()).as_deref(), Some(png));
        }

        // The RTFD: what was said, styled, with each screenshot where it was made.
        let rtfd = items[0].dataForType(unsafe { NSPasteboardTypeRTFD }).expect("RTFD");
        let read =
            unsafe { NSAttributedString::initWithRTFD_documentAttributes(NSAttributedString::alloc(), &rtfd, None) }
                .expect("readable RTFD");
        assert_eq!(
            read.string().to_string(),
            "In: Safari (\"Shop\")\n\nThe Save button [1] sits low.\n\n\u{fffc}\n\n1. button #save\nFeedback: Line it up.\n\n\u{fffc}"
        );
        let mut attachments = Vec::new();
        let font =
            |at: usize| unsafe { read.attribute_atIndex_effectiveRange(NSFontAttributeName, at, std::ptr::null_mut()) };
        for (at, c) in read.string().to_string().encode_utf16().enumerate() {
            if c == 0xfffc {
                let attachment = unsafe {
                    read.attribute_atIndex_effectiveRange(NSAttachmentAttributeName, at, std::ptr::null_mut())
                };
                let attachment = attachment.and_then(|a| a.downcast::<NSTextAttachment>().ok()).expect("an attachment");
                let wrapper = attachment.fileWrapper().expect("its file");
                attachments.push((
                    wrapper.preferredFilename().map(|n| n.to_string()),
                    wrapper.regularFileContents().map(|d| d.to_vec()),
                ));
            }
        }
        assert_eq!(
            attachments,
            [
                (Some("Screenshot 1.png".into()), Some(RED.to_vec())),
                (Some("Screenshot 2.png".into()), Some(BLUE.to_vec())),
            ]
        );
        let font_name =
            |at: usize| font(at).and_then(|f| f.downcast::<NSFont>().ok()).map(|f| f.fontName().to_string());
        let bold = font_name(0).expect("a font");
        let regular = font_name(read.string().to_string().find("Safari").unwrap()).expect("a font");
        let code = font_name(read.string().to_string().find("#save").unwrap()).expect("a font");
        assert!(bold != regular && code != regular && code != bold, "{bold}, {regular}, {code}");
        // Named pasteboards stay until released (objc2 doesn't bind it).
        if std::env::var_os("ASIDES_COPY_DUMP").is_none() {
            let _: () = unsafe { objc2::msg_send![&*pasteboard, releaseGlobally] };
        }
    }
}
