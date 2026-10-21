//! What an app can put on the screen besides the full frame: [`Area`] (see [`crate::App::area`])
//! and [`MenuSetting`] (see [`crate::App::menu_settings`]).

use embedded_graphics::{prelude::*, primitives::Rectangle};

/// The part of the screen an app draws when it doesn't use all of it: a framebuffer of `size`,
/// shown at `screen`. If `screen` is larger, the framebuffer is scaled up on the way to the
/// display; where a screen pixel falls between two of the framebuffer's, it gets their average.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub size: Size,
    pub screen: Rectangle,
}

impl Area {
    /// 1:1, at `screen`.
    pub const fn at(screen: Rectangle) -> Self {
        Area {
            size: screen.size,
            screen,
        }
    }

    /// A framebuffer of `size`, scaled up to fill `screen`.
    pub const fn scaled(size: Size, screen: Rectangle) -> Self {
        Area { size, screen }
    }
}

/// One of an app's own settings, shown in the pause menu below the volume, e.g.
/// `Colours: Green`. Left and Right (or A) change it, through [`crate::App::change_menu_setting`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSetting {
    pub label: &'static str,
    pub value: String,
}
