//! Compositor-side payload for smithay's `image_capture_source`. All dispatch
//! boilerplate and manager states come from smithay; only `SourceKind` lives here.

use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Physical, Size};

/// Stashed in `ImageCaptureSource::user_data()` at create time so the renderer
/// can match on it to decide what to draw into the buffer.
///
/// Toplevel `initial_size` is captured at source-creation time so sessions can
/// advertise `buffer_size` without space access. Resizes mid-capture are not
/// propagated yet.
#[derive(Debug, Clone)]
pub enum SourceKind {
    Output(Output),
    Toplevel {
        surface: WlSurface,
        initial_size: Size<i32, Physical>,
    },
    /// Toplevel handle was dead by source-creation time, or its surface
    /// vanished. Capture frames fail.
    Destroyed,
}
