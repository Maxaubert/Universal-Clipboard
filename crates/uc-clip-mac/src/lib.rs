//! macOS clipboard backend (plan 3.9, milestone M5).
//!
//! [`MacClipboard`] implements the [`uc_clip::Clipboard`] trait on an `NSPasteboard`:
//! - It works on a pasteboard chosen by name. The app uses the general pasteboard. Tests use a
//!   private named pasteboard, so they never touch the user's clipboard and show no prompt.
//! - Change detection is polling of `changeCount` ([`poll::Poller`] picks the wait).
//! - Read: `public.utf8-plain-text`, `public.html`, `public.png`, else `public.tiff` turned
//!   into PNG. Clips with a private marker, a file reference or our own marker are sorted by
//!   their types alone, and their content is not read.
//! - Write: text, HTML, PNG plus a TIFF copy, and our marker, with
//!   `prepareForNewContents(.currentHostOnly)` so Apple's Universal Clipboard does not send our
//!   clips again.
//! - A clip that came from another Apple device through Universal Clipboard is flagged
//!   ([`uc_clip::Snapshot::remote`]). Plan decision 8 decides what the agent does with it.
//!
//! The OS code builds only on macOS. The pure parts ([`poll`], [`types`]) build and are tested
//! on every OS.

pub mod poll;
pub mod types;

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac;

#[cfg(target_os = "macos")]
pub use mac::{AccessBehavior, MacClipboard, TEST_PASTEBOARD_ENV};
