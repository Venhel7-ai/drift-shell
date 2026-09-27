use smithay::desktop::Window;
use smithay::utils::{IsAlive, Logical, Size};
use smithay::wayland::seat::WaylandFocus;

use crate::window_ext::WindowExt;

/// The seam between [`Stage`](super::Stage) and the window type it manages.
/// Stage logic only ever queries windows through this trait (plus `WindowExt`),
/// so tests can drive the same logic with a mock. Add methods only when a real
/// callsite demands one.
pub trait StageElement: WindowExt + Clone + PartialEq {
    fn size(&self) -> Size<i32, Logical>;
    /// Named `is_alive` (not `alive`) to avoid ambiguity with smithay's
    /// `IsAlive::alive` at callsites that have both traits in scope.
    fn is_alive(&self) -> bool;
    /// True when `self`'s toplevel parent (xdg_toplevel.set_parent) is `parent`.
    fn is_child_of(&self, parent: &Self) -> bool;
}

impl StageElement for Window {
    fn size(&self) -> Size<i32, Logical> {
        self.geometry().size
    }

    fn is_alive(&self) -> bool {
        IsAlive::alive(self)
    }

    fn is_child_of(&self, parent: &Self) -> bool {
        parent
            .wl_surface()
            .is_some_and(|s| self.parent_surface().as_ref() == Some(&*s))
    }
}
