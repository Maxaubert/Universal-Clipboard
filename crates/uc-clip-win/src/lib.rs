//! Windows clipboard backend (plan 3.9, 3.10, milestone M4).
//!
//! [`WinClipboard`] implements [`uc_clip::Clipboard`]:
//! - **Watch:** `AddClipboardFormatListener` on a message-only window, so change events come
//!   from Windows (no polling). See [`WinClipboard::changes`].
//! - **Read:** `CF_UNICODETEXT`; `HTML Format` (fragment only); the registered `PNG` format,
//!   else `CF_DIBV5`, else `CF_DIB`, converted to PNG.
//! - **Skip:** private copies (`ExcludeClipboardContentFromMonitorProcessing`,
//!   `CanIncludeInClipboardHistory = 0`, `CanUploadToCloudClipboard = 0`) and file lists
//!   (`CF_HDROP`, `FileGroupDescriptorW`). Their content is never read.
//! - **Write:** text, `PNG` plus `CF_DIBV5`, our origin marker `UniversalClipboard.Origin` and
//!   `CanUploadToCloudClipboard = 0`, in one open and close.
//!
//! The format conversions ([`dib`], [`html`], [`formats`]) have no OS calls and are tested on
//! every OS. The real clipboard tests run inside a private window station made by
//! `tools/winhost`, so they never touch the user's clipboard.

pub mod dib;
pub mod formats;
pub mod html;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{RawFormat, Retry, WinClipboard, format_name, register_format};
