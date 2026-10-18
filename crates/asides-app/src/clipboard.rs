//! The general pasteboard, while an aside's screenshots are pasted into a session (Ctrl+V): a
//! screenshot put on it, and what the user had there kept, to put back.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypePNG, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};

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

/// Puts a PNG on the pasteboard, as a screenshot is copied.
pub fn put_png(bytes: &[u8]) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setData_forType(Some(&NSData::with_bytes(bytes)), unsafe { NSPasteboardTypePNG });
}
