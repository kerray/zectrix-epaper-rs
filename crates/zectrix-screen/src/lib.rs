//! Interactive screens for the ZECTRIX NOTE4 (mono): on-device menus, and the device side
//! of the screen protocol in `docs/INTERACTIVE.md` ("Protocol (draft v2)").
//!
//! * [`menu`]: a static tree of menus with a Back row on top of each submenu, walked with
//!   the button events; items open submenus or hand an app-defined action to the app.
//! * [`view`]: drawing menus, title bars and simple text screens with `embedded-graphics`.
//! * [`present`]: showing a frame with one partial refresh of whatever changed, or a full
//!   refresh when the policy asks for one.
//! * [`bundle`]: the `application/x-note4-screen` bundle (base frame + one highlight strip
//!   per row). [`Screen`] works in place on the downloaded bytes, moves the cursor and
//!   applies patches; it writes the result into a [`MonoFrame`] and returns the rectangles
//!   to refresh.
//! * [`patch`]: the `application/x-note4-patch` body, parsed without copying.
//! * [`policy`]: when a partial refresh is allowed and when a full refresh is due
//!   (ghosting).
//! * [`buttons`]: debounce and short/held presses of Up, Down and OK, turned into
//!   [`Event`]s. Time is passed in.
//! * [`session`]: the idle timer that ends an interactive session.
//!
//! The firmware owns the buffers: the bundle bytes (e.g. in PSRAM), the target frame and
//! the driver's shadow (`ssd2683::Ssd2683::show_mono_partial`). Nothing here allocates.
//! Only [`present`] touches the driver, through the [`MonoPanel`] trait.

#![cfg_attr(not(test), no_std)]

pub mod bundle;
pub mod buttons;
pub mod menu;
pub mod patch;
pub mod policy;
pub mod present;
pub mod session;
pub mod view;

mod blit;

pub use bundle::{BundleError, Dirty, Layout, Merge, Move, Screen};
pub use buttons::{Button, ButtonConfig, Buttons, Event};
pub use patch::{Patch, PatchError};
pub use policy::{RefreshKind, RefreshPolicy, Update};
pub use present::{MonoPanel, Presenter, Shown};
pub use session::Session;
pub use ssd2683::{MonoFrame, Rect};

/// MIME type of a screen bundle.
pub const SCREEN_CONTENT_TYPE: &str = "application/x-note4-screen";
/// MIME type of a patch.
pub const PATCH_CONTENT_TYPE: &str = "application/x-note4-patch";
