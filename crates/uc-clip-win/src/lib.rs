//! Windows clipboard backend (plan 3.9, 3.10, milestone M4).
//!
//! [`WinClipboard`] implements [`uc_clip::Clipboard`]:
//! - **Watch:** `AddClipboardFormatListener` on a message-only window, so change events come
//!   from Windows (no polling). See [`WinClipboard::changes`].
//! - **Read:** `CF_UNICODETEXT`; `HTML Format` (fragment only); the registered `PNG` format,
//!   else `CF_DIBV5`, else `CF_DIB`, converted to PNG.
//! - **Files:** `CF_HDROP` gives the paths of a file copy (wide lists only), and then text and
//!   images next to it are not read. `FileGroupDescriptorW` without `CF_HDROP` is a virtual
//!   file copy. `cloud_only` tells whether a path is a cloud placeholder (spec 2.4).
//! - **Skip:** private copies (`ExcludeClipboardContentFromMonitorProcessing`,
//!   `CanIncludeInClipboardHistory = 0`, `CanUploadToCloudClipboard = 0`). Their content is
//!   never read.
//! - **Write:** text, `PNG` plus `CF_DIBV5`, our origin marker `UniversalClipboard.Origin` and
//!   `CanUploadToCloudClipboard = 0`, in one open and close. File writes put `CF_HDROP` and
//!   `Preferred DropEffect = 1` (copy) instead of text, and an optional image next to them.
//!
//! The format conversions ([`dib`], [`html`], [`formats`], [`files`]) have no OS calls and are
//! tested on every OS. The real clipboard tests run inside a private window station made by
//! `tools/winhost`, so they never touch the user's clipboard.

pub mod dib;
pub mod files;
pub mod formats;
pub mod html;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{
    READ_TIMEOUT, RawFormat, Retry, WinClipboard, cloud_only, format_name, register_format,
};
