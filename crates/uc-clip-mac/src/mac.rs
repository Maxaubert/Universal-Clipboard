//! The `NSPasteboard` code. macOS only.

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{msg_send, sel};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardAccessBehavior,
    NSPasteboardContentsOptions,
};
use objc2_foundation::{NSData, NSDictionary, NSString};
use uc_clip::{Clipboard, ClipboardError, Format, RawPart, Snapshot};

use crate::types::{self, Kinds};

/// Test builds set this to the name of a private pasteboard (plan 6.4). [`MacClipboard::from_env`]
/// then uses that pasteboard in place of the general one.
pub const TEST_PASTEBOARD_ENV: &str = "UCLIP_TEST_PASTEBOARD";

/// How many times a read starts again when the clipboard changes while it is read.
const READ_TRIES: u32 = 3;

/// The "Paste from Other Apps" setting of this app (macOS 15.4 and later, plan 3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessBehavior {
    /// The app never triggered a prompt. On the general pasteboard the first read will ask.
    Default,
    /// Every read asks the user.
    Ask,
    /// Reads are allowed without a prompt.
    AlwaysAllow,
    /// Reads are refused without a prompt.
    AlwaysDeny,
    /// A value this build does not know.
    Unknown(isize),
}

/// One macOS pasteboard behind the [`Clipboard`] trait.
///
/// Not `Send`: create it on the thread that uses it.
pub struct MacClipboard {
    pb: Retained<NSPasteboard>,
    read_remote_content: bool,
}

impl std::fmt::Debug for MacClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacClipboard")
            .field("name", &self.name())
            .finish()
    }
}

impl MacClipboard {
    /// The general pasteboard: the user's clipboard. Only the app uses this, never a test.
    pub fn general() -> Self {
        Self::wrap(NSPasteboard::generalPasteboard())
    }

    /// A pasteboard by name. A name other than the general one gives a private pasteboard that
    /// no other app looks at. Reading it shows no prompt.
    pub fn named(name: &str) -> Self {
        Self::wrap(NSPasteboard::pasteboardWithName(&NSString::from_str(name)))
    }

    /// The pasteboard named in `UCLIP_TEST_PASTEBOARD` if it is set and not empty, else the
    /// general pasteboard.
    pub fn from_env() -> Self {
        match std::env::var(TEST_PASTEBOARD_ENV) {
            Ok(name) if !name.is_empty() => Self::named(&name),
            _ => Self::general(),
        }
    }

    fn wrap(pb: Retained<NSPasteboard>) -> Self {
        Self {
            pb,
            read_remote_content: true,
        }
    }

    /// The pasteboard's name.
    pub fn name(&self) -> String {
        autoreleasepool(|_| self.pb.name().to_string())
    }

    /// The change counter. A cheap call: it reads no content and shows no prompt.
    pub fn change_count(&self) -> u64 {
        to_u64(self.pb.changeCount())
    }

    /// Whether [`Clipboard::read`] reads the content of a clip that came through Apple's
    /// Universal Clipboard (default: yes). Reading it makes macOS fetch the data from the other
    /// device. With `false` the snapshot of such a clip has the `remote` flag and no parts.
    pub fn set_read_remote_content(&mut self, on: bool) {
        self.read_remote_content = on;
    }

    /// The "Paste from Other Apps" setting, or `None` before macOS 15.4.
    pub fn access_behavior(&self) -> Option<AccessBehavior> {
        if !self.pb.respondsToSelector(sel!(accessBehavior)) {
            return None;
        }
        Some(match self.pb.accessBehavior() {
            NSPasteboardAccessBehavior::Default => AccessBehavior::Default,
            NSPasteboardAccessBehavior::Ask => AccessBehavior::Ask,
            NSPasteboardAccessBehavior::AlwaysAllow => AccessBehavior::AlwaysAllow,
            NSPasteboardAccessBehavior::AlwaysDeny => AccessBehavior::AlwaysDeny,
            other => AccessBehavior::Unknown(other.0),
        })
    }

    /// The pasteboard types now present. Reads no content.
    pub fn types(&self) -> Option<Vec<String>> {
        autoreleasepool(|_| {
            self.pb
                .types()
                .map(|a| a.iter().map(|t| t.to_string()).collect())
        })
    }

    /// Free a named pasteboard in the pasteboard server. Tests call this when they end.
    /// Does nothing to the general pasteboard.
    pub fn release_globally(self) {
        if self.name() == GENERAL_NAME {
            return;
        }
        // SAFETY: `releaseGlobally` takes no arguments and returns void. It exists on every
        // macOS version that AppKit supports.
        let _: () = unsafe { msg_send![&*self.pb, releaseGlobally] };
    }

    fn data(&self, ty: &str) -> Option<Vec<u8>> {
        self.pb
            .dataForType(&NSString::from_str(ty))
            .map(|d| d.to_vec())
    }

    fn read_once(&self) -> Result<Snapshot, ClipboardError> {
        let seq = self.change_count();
        let Some(types) = self.types() else {
            // The type list could not be read, so a private marker could not be ruled out
            // (plan 3.10): the clip counts as private.
            return Ok(Snapshot {
                seq,
                private: true,
                ..Snapshot::default()
            });
        };
        let kinds = Kinds::of(&types);
        let mut snap = Snapshot {
            seq,
            parts: Vec::new(),
            has_marker: kinds.marker,
            private: kinds.private,
            has_files: kinds.files,
            remote: kinds.remote,
        };
        if kinds.skip_content() || (kinds.remote && !self.read_remote_content) {
            return Ok(snap);
        }
        let has = |t: &str| types.iter().any(|x| x == t);
        // `stringForType` also turns older text types (UTF-16, `NSStringPboardType`) into a
        // string, so it is asked even if the UTF-8 type is not listed.
        if let Some(s) = self.pb.stringForType(&NSString::from_str(types::TEXT)) {
            snap.parts
                .push((Format::TextPlain, s.to_string().into_bytes()));
        }
        if has(types::HTML)
            && let Some(html) = self.data(types::HTML).and_then(|b| types::html_to_utf8(&b))
        {
            snap.parts.push((Format::TextHtml, html));
        }
        if has(types::PNG)
            && let Some(png) = self.data(types::PNG)
        {
            snap.parts.push((Format::ImagePng, png));
        } else if has(types::TIFF)
            && let Some(png) = self.data(types::TIFF).and_then(|t| tiff_to_png(&t))
        {
            snap.parts.push((Format::ImagePng, png));
        }
        Ok(snap)
    }
}

impl Clipboard for MacClipboard {
    fn read(&mut self) -> Result<Snapshot, ClipboardError> {
        for _ in 0..READ_TRIES {
            let snap = autoreleasepool(|_| self.read_once())?;
            // Another app wrote while we read: the parts may come from two clips.
            if self.change_count() == snap.seq {
                return Ok(snap);
            }
        }
        Err(ClipboardError::Busy)
    }

    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError> {
        autoreleasepool(|_| {
            self.pb
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            for (format, bytes) in parts {
                let ok = match format {
                    Format::TextPlain => {
                        let text = std::str::from_utf8(bytes)
                            .map_err(|_| ClipboardError::Other("text-not-utf8".into()))?;
                        self.pb.setString_forType(
                            &NSString::from_str(text),
                            &NSString::from_str(types::TEXT),
                        )
                    }
                    Format::TextHtml => self.set_data(types::HTML, bytes),
                    Format::ImagePng => {
                        // Many Mac apps only paste TIFF, so a TIFF copy goes next to the PNG.
                        let tiff_ok = match png_to_tiff(bytes) {
                            Some(tiff) => self.set_data(types::TIFF, &tiff),
                            None => true,
                        };
                        self.set_data(types::PNG, bytes) && tiff_ok
                    }
                };
                if !ok {
                    return Err(ClipboardError::Other(format!("set-{}", format.mime())));
                }
            }
            if !self.set_data(types::MARKER, &[]) {
                return Err(ClipboardError::Other("set-marker".into()));
            }
            Ok(self.change_count())
        })
    }
}

impl MacClipboard {
    fn set_data(&self, ty: &str, bytes: &[u8]) -> bool {
        self.pb
            .setData_forType(Some(&NSData::with_bytes(bytes)), &NSString::from_str(ty))
    }

    /// Put raw types on the pasteboard, as another app would. Test helper.
    #[cfg(test)]
    fn set_raw(&self, items: &[(&str, &[u8])]) -> u64 {
        autoreleasepool(|_| {
            self.pb.clearContents();
            for (ty, bytes) in items {
                assert!(self.set_data(ty, bytes), "set {ty}");
            }
            self.change_count()
        })
    }
}

/// The name of the general pasteboard (`NSPasteboardNameGeneral`). A constant, so that tests
/// can check it without opening the general pasteboard.
const GENERAL_NAME: &str = "Apple CFPasteboard general";

fn to_u64(n: isize) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// Turn TIFF bytes into PNG bytes with AppKit. Only the first image of a multi-page TIFF.
fn tiff_to_png(tiff: &[u8]) -> Option<Vec<u8>> {
    autoreleasepool(|_| {
        let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(tiff))?;
        let props = NSDictionary::<NSString, AnyObject>::new();
        // SAFETY: the properties dictionary is empty, so its value type does not matter.
        let png =
            unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
        Some(png.to_vec())
    })
}

/// Turn PNG bytes into TIFF bytes with AppKit.
fn png_to_tiff(png: &[u8]) -> Option<Vec<u8>> {
    autoreleasepool(|_| {
        let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(png))?;
        rep.TIFFRepresentation().map(|d| d.to_vec())
    })
}

#[cfg(test)]
mod tests;
