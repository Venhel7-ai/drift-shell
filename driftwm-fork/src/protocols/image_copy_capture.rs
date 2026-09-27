use std::sync::Mutex;
use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::{
    ext_image_copy_capture_cursor_session_v1, ext_image_copy_capture_frame_v1,
    ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Physical, Size};
use smithay::wayland::image_capture_source::ImageCaptureSource;

use ext_image_copy_capture_cursor_session_v1::ExtImageCopyCaptureCursorSessionV1;
use ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1;
use ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;
use ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1;

use super::image_capture_source::SourceKind;

const VERSION: u32 = 1;

/// What a pending capture should render — output (full screen) or a single
/// window's surface tree.
#[derive(Debug, Clone)]
pub enum PendingCaptureKind {
    Output(Output),
    Toplevel(WlSurface),
}

/// A pending capture ready to be fulfilled by the render loop.
pub struct PendingCapture {
    pub frame: ExtImageCopyCaptureFrameV1,
    pub buffer: WlBuffer,
    pub kind: PendingCaptureKind,
    pub paint_cursors: bool,
    pub buffer_size: Size<i32, Physical>,
}

/// Per-session state stored by the compositor.
struct SessionData {
    source: SourceKind,
    paint_cursors: bool,
    buffer_size: Size<i32, Physical>,
    has_active_frame: bool,
    stopped: bool,
    waiting_frame: Option<WaitingFrame>,
    has_captured_once: bool,
    /// Last promotion time, for `max_capture_fps` rate-limiting.
    last_frame_time: Option<Duration>,
}

struct WaitingFrame {
    frame: ExtImageCopyCaptureFrameV1,
    buffer: WlBuffer,
}

/// Mutable frame state, wrapped in Mutex for interior mutability.
pub struct CaptureFrameData {
    pub session: ExtImageCopyCaptureSessionV1,
    buffer: Option<WlBuffer>,
    captured: bool,
}

pub struct ImageCopyCaptureState {
    sessions: Vec<(ExtImageCopyCaptureSessionV1, SessionData)>,
}

pub struct ImageCopyCaptureGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

/// Rate-limits continuous captures: true when `min_interval` has not elapsed
/// since `last`. Uncapped or first frame → false.
fn capture_too_soon(last: Option<Duration>, now: Duration, min_interval: Option<Duration>) -> bool {
    match (min_interval, last) {
        (Some(iv), Some(last)) => now.saturating_sub(last) < iv,
        _ => false,
    }
}

impl ImageCopyCaptureState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ExtImageCopyCaptureManagerV1, ImageCopyCaptureGlobalData>,
        D: Dispatch<ExtImageCopyCaptureManagerV1, ()>,
        D: Dispatch<ExtImageCopyCaptureSessionV1, ()>,
        D: Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>>,
        D: Dispatch<ExtImageCopyCaptureCursorSessionV1, ()>,
        D: ImageCopyCaptureHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ImageCopyCaptureGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ExtImageCopyCaptureManagerV1, _>(VERSION, global_data);
        Self {
            sessions: Vec::new(),
        }
    }

    /// Promote waiting frames to pending captures for an output that has new
    /// damage. A session whose last frame is newer than `min_interval` is skipped;
    /// its frame stays parked and retries on a later render.
    pub fn promote_waiting_frames(
        &mut self,
        output: &Output,
        pending: &mut Vec<PendingCapture>,
        now: Duration,
        min_interval: Option<Duration>,
    ) {
        for (_session_obj, session) in &mut self.sessions {
            if session.stopped {
                continue;
            }
            let SourceKind::Output(ref session_output) = session.source else {
                continue;
            };
            if session_output != output {
                continue;
            }
            if session.waiting_frame.is_none()
                || capture_too_soon(session.last_frame_time, now, min_interval)
            {
                continue;
            }
            let waiting = session.waiting_frame.take().unwrap();
            session.last_frame_time = Some(now);
            pending.push(PendingCapture {
                frame: waiting.frame,
                buffer: waiting.buffer,
                kind: PendingCaptureKind::Output(output.clone()),
                paint_cursors: session.paint_cursors,
                buffer_size: session.buffer_size,
            });
        }
    }

    /// Mark a session's active frame as completed.
    pub fn frame_done(&mut self, session_obj: &ExtImageCopyCaptureSessionV1) {
        if let Some((_, session)) = self.sessions.iter_mut().find(|(s, _)| s == session_obj) {
            session.has_active_frame = false;
            session.has_captured_once = true;
        }
    }

    /// Send stopped to all sessions capturing a given output, and remove them.
    pub fn remove_output(&mut self, output: &Output) {
        self.sessions.retain(|(session_obj, session)| {
            let SourceKind::Output(ref session_output) = session.source else {
                return true;
            };
            if session_output == output {
                if session_obj.is_alive() {
                    session_obj.stopped();
                }
                false
            } else {
                true
            }
        });
    }

    /// Send stopped to all sessions capturing a given toplevel surface, and
    /// remove them. Call when a window closes.
    pub fn remove_toplevel(&mut self, surface: &WlSurface) {
        self.sessions.retain(|(session_obj, session)| {
            let SourceKind::Toplevel {
                surface: ref session_surface,
                ..
            } = session.source
            else {
                return true;
            };
            if session_surface == surface {
                if session_obj.is_alive() {
                    session_obj.stopped();
                }
                false
            } else {
                true
            }
        });
    }

    /// Promote waiting toplevel-capture frames to pending captures. Toplevel
    /// captures have no damage tracking, so absent a `min_interval` cap they
    /// promote on every render; `min_interval` rate-limits them, parking a
    /// too-recent session's frame for a later render.
    pub fn promote_waiting_toplevel_frames(
        &mut self,
        pending: &mut Vec<PendingCapture>,
        now: Duration,
        min_interval: Option<Duration>,
    ) {
        for (_session_obj, session) in &mut self.sessions {
            if session.stopped {
                continue;
            }
            let SourceKind::Toplevel { ref surface, .. } = session.source else {
                continue;
            };
            if !surface.is_alive() {
                continue;
            }
            if session.waiting_frame.is_none()
                || capture_too_soon(session.last_frame_time, now, min_interval)
            {
                continue;
            }
            let waiting = session.waiting_frame.take().unwrap();
            session.last_frame_time = Some(now);
            pending.push(PendingCapture {
                frame: waiting.frame,
                buffer: waiting.buffer,
                kind: PendingCaptureKind::Toplevel(surface.clone()),
                paint_cursors: session.paint_cursors,
                buffer_size: session.buffer_size,
            });
        }
    }

    /// Clean up dead sessions.
    pub fn cleanup(&mut self) {
        self.sessions.retain(|(s, _)| s.is_alive());
    }
}

// --- Handler trait ---

pub trait ImageCopyCaptureHandler {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState;
    fn capture_frame(&mut self, capture: PendingCapture);

    /// DRM render-node `dev_t` and supported DMA-BUF formats. Returning `None`
    /// leaves SHM as the only buffer path — modern clients (xdph-cosmic) skip
    /// SHM entirely and will fail unless DMA-BUF is advertised.
    fn dmabuf_constraints(&self) -> Option<(u64, smithay::backend::allocator::format::FormatSet)> {
        None
    }
}

// --- GlobalDispatch: manager ---

impl<D> GlobalDispatch<ExtImageCopyCaptureManagerV1, ImageCopyCaptureGlobalData, D>
    for ImageCopyCaptureState
where
    D: GlobalDispatch<ExtImageCopyCaptureManagerV1, ImageCopyCaptureGlobalData>,
    D: Dispatch<ExtImageCopyCaptureManagerV1, ()>,
    D: Dispatch<ExtImageCopyCaptureSessionV1, ()>,
    D: Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>>,
    D: Dispatch<ExtImageCopyCaptureCursorSessionV1, ()>,
    D: ImageCopyCaptureHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<ExtImageCopyCaptureManagerV1>,
        _global_data: &ImageCopyCaptureGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &ImageCopyCaptureGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

// --- Dispatch: manager requests ---

impl<D> Dispatch<ExtImageCopyCaptureManagerV1, (), D> for ImageCopyCaptureState
where
    D: Dispatch<ExtImageCopyCaptureManagerV1, ()>,
    D: Dispatch<ExtImageCopyCaptureSessionV1, ()>,
    D: Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>>,
    D: Dispatch<ExtImageCopyCaptureCursorSessionV1, ()>,
    D: ImageCopyCaptureHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession {
                session,
                source,
                options,
            } => {
                let kind = ImageCaptureSource::from_resource(&source)
                    .and_then(|s| s.user_data().get::<SourceKind>().cloned())
                    .unwrap_or(SourceKind::Destroyed);
                let paint_cursors = options.into_result().is_ok_and(|o| {
                    o.contains(ext_image_copy_capture_manager_v1::Options::PaintCursors)
                });

                let buffer_size = match &kind {
                    // Raw mode size in buffer pixel (scanout) coordinates. The frame's
                    // `transform` event lets the client orient it; advertising the
                    // transformed size here would skew rotated outputs.
                    SourceKind::Output(output) => output
                        .current_mode()
                        .map(|m| m.size)
                        .unwrap_or((1, 1).into()),
                    SourceKind::Toplevel { initial_size, .. } => *initial_size,
                    SourceKind::Destroyed => (1, 1).into(),
                };

                let dmabuf_info = state.dmabuf_constraints();

                let session_obj = data_init.init(session, ());

                // Send buffer constraints. Order between shm_format / dmabuf_*
                // doesn't matter per protocol; buffer_size and done are required.
                session_obj.buffer_size(buffer_size.w as u32, buffer_size.h as u32);
                session_obj.shm_format(Format::Xrgb8888);
                session_obj.shm_format(Format::Argb8888);

                if let Some((dev_t, formats)) = dmabuf_info {
                    session_obj.dmabuf_device(dev_t.to_ne_bytes().to_vec());

                    // Group modifiers by fourcc — protocol expects one event
                    // per fourcc with all valid modifiers packed as u64s.
                    // BTreeMap so emission order is stable across sessions
                    // (helps when diffing wireshark/debug traces).
                    let mut by_fourcc: std::collections::BTreeMap<u32, Vec<u64>> =
                        std::collections::BTreeMap::new();
                    for fmt in formats.iter() {
                        by_fourcc
                            .entry(fmt.code as u32)
                            .or_default()
                            .push(u64::from(fmt.modifier));
                    }
                    for (fourcc, mods) in by_fourcc {
                        let mods_bytes: Vec<u8> =
                            mods.iter().flat_map(|m| m.to_ne_bytes()).collect();
                        session_obj.dmabuf_format(fourcc, mods_bytes);
                    }
                }
                session_obj.done();

                // Source already gone by the time the session was created —
                // accept it but immediately stop, so the client cleans up.
                let stopped = matches!(kind, SourceKind::Destroyed)
                    || matches!(&kind, SourceKind::Toplevel { surface, .. } if !surface.is_alive());
                if stopped {
                    session_obj.stopped();
                }

                let cap_state = state.image_copy_capture_state();
                cap_state.sessions.push((
                    session_obj,
                    SessionData {
                        source: kind,
                        paint_cursors,
                        buffer_size,
                        has_active_frame: false,
                        stopped,
                        waiting_frame: None,
                        has_captured_once: false,
                        last_frame_time: None,
                    },
                ));
            }
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession {
                session,
                ..
            } => {
                // Stub: cursor sessions not yet supported
                data_init.init(session, ());
            }
            ext_image_copy_capture_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

// --- Dispatch: session requests ---

impl<D> Dispatch<ExtImageCopyCaptureSessionV1, (), D> for ImageCopyCaptureState
where
    D: Dispatch<ExtImageCopyCaptureSessionV1, ()>,
    D: Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>>,
    D: ImageCopyCaptureHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        session: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_image_copy_capture_session_v1::Request::CreateFrame { frame } => {
                let cap_state = state.image_copy_capture_state();
                let session_entry = cap_state.sessions.iter_mut().find(|(s, _)| s == session);

                if let Some((_, session_data)) = session_entry {
                    if session_data.has_active_frame {
                        session.post_error(
                            ext_image_copy_capture_session_v1::Error::DuplicateFrame,
                            "create_frame sent before destroying previous frame",
                        );
                        return;
                    }
                    session_data.has_active_frame = true;
                }

                data_init.init(
                    frame,
                    Mutex::new(CaptureFrameData {
                        session: session.clone(),
                        buffer: None,
                        captured: false,
                    }),
                );
            }
            ext_image_copy_capture_session_v1::Request::Destroy => {
                let cap_state = state.image_copy_capture_state();
                if let Some((_, session_data)) =
                    cap_state.sessions.iter_mut().find(|(s, _)| s == session)
                {
                    session_data.stopped = true;
                    session_data.waiting_frame = None;
                }
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        session: &ExtImageCopyCaptureSessionV1,
        _data: &(),
    ) {
        let cap_state = state.image_copy_capture_state();
        cap_state.sessions.retain(|(s, _)| s != session);
    }
}

// --- Dispatch: frame requests ---

impl<D> Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>, D> for ImageCopyCaptureState
where
    D: Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>>,
    D: ImageCopyCaptureHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        frame: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &Mutex<CaptureFrameData>,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_image_copy_capture_frame_v1::Request::Destroy => {}
            ext_image_copy_capture_frame_v1::Request::AttachBuffer { buffer } => {
                let mut fd = data.lock().unwrap();
                if fd.captured {
                    frame.post_error(
                        ext_image_copy_capture_frame_v1::Error::AlreadyCaptured,
                        "attach_buffer after capture",
                    );
                    return;
                }
                fd.buffer = Some(buffer);
            }
            ext_image_copy_capture_frame_v1::Request::DamageBuffer { .. } => {
                let fd = data.lock().unwrap();
                if fd.captured {
                    frame.post_error(
                        ext_image_copy_capture_frame_v1::Error::AlreadyCaptured,
                        "damage_buffer after capture",
                    );
                }
                // Accept all damage — we always render full frames
            }
            ext_image_copy_capture_frame_v1::Request::Capture => {
                let mut fd = data.lock().unwrap();
                if fd.captured {
                    frame.post_error(
                        ext_image_copy_capture_frame_v1::Error::AlreadyCaptured,
                        "capture already requested",
                    );
                    return;
                }
                let Some(buffer) = fd.buffer.take() else {
                    frame.post_error(
                        ext_image_copy_capture_frame_v1::Error::NoBuffer,
                        "no buffer attached",
                    );
                    return;
                };
                fd.captured = true;
                let session_obj = fd.session.clone();
                drop(fd);

                // Find the session to get output + paint_cursors
                let cap_state = state.image_copy_capture_state();
                let session_entry = cap_state
                    .sessions
                    .iter_mut()
                    .find(|(s, _)| *s == session_obj);

                let Some((_, session_data)) = session_entry else {
                    // Session gone — fail the frame
                    frame.failed(ext_image_copy_capture_frame_v1::FailureReason::Unknown);
                    return;
                };

                if session_data.stopped {
                    frame.failed(ext_image_copy_capture_frame_v1::FailureReason::Stopped);
                    return;
                }

                let kind = match &session_data.source {
                    SourceKind::Output(o) => PendingCaptureKind::Output(o.clone()),
                    SourceKind::Toplevel { surface, .. } if surface.is_alive() => {
                        PendingCaptureKind::Toplevel(surface.clone())
                    }
                    SourceKind::Toplevel { .. } | SourceKind::Destroyed => {
                        // Source vanished between session creation and this
                        // capture request — terminate the session.
                        session_data.stopped = true;
                        if session_obj.is_alive() {
                            session_obj.stopped();
                        }
                        frame.failed(ext_image_copy_capture_frame_v1::FailureReason::Stopped);
                        return;
                    }
                };

                // First capture: render immediately. Subsequent: wait for damage
                // (output) or wait for next frame (toplevel).
                if !session_data.has_captured_once {
                    let capture = PendingCapture {
                        frame: frame.clone(),
                        buffer,
                        kind,
                        paint_cursors: session_data.paint_cursors,
                        buffer_size: session_data.buffer_size,
                    };
                    state.capture_frame(capture);
                } else {
                    // Queue for next damage
                    let cap_state = state.image_copy_capture_state();
                    let session_entry = cap_state
                        .sessions
                        .iter_mut()
                        .find(|(s, _)| *s == session_obj);
                    if let Some((_, session_data)) = session_entry {
                        session_data.waiting_frame = Some(WaitingFrame {
                            frame: frame.clone(),
                            buffer,
                        });
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        _frame: &ExtImageCopyCaptureFrameV1,
        data: &Mutex<CaptureFrameData>,
    ) {
        let fd = data.lock().unwrap();
        let session_obj = fd.session.clone();
        drop(fd);

        let cap_state = state.image_copy_capture_state();
        if let Some((_, session_data)) = cap_state
            .sessions
            .iter_mut()
            .find(|(s, _)| *s == session_obj)
        {
            session_data.has_active_frame = false;
            // Also clear any waiting frame that references this destroyed frame
            session_data.waiting_frame = None;
        }
    }
}

// --- Dispatch: cursor session (stub) ---

impl<D> Dispatch<ExtImageCopyCaptureCursorSessionV1, (), D> for ImageCopyCaptureState
where
    D: Dispatch<ExtImageCopyCaptureSessionV1, ()>,
    D: Dispatch<ExtImageCopyCaptureFrameV1, Mutex<CaptureFrameData>>,
    D: ImageCopyCaptureHandler,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _session: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_image_copy_capture_cursor_session_v1::Request::Destroy => {}
            ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } => {
                // Create a session object but immediately stop it
                let session_obj = data_init.init(session, ());
                session_obj.stopped();
            }
            _ => unreachable!(),
        }
    }
}

// --- Delegate macro ---

#[macro_export]
macro_rules! delegate_image_copy_capture {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1: $crate::protocols::image_copy_capture::ImageCopyCaptureGlobalData
        ] => $crate::protocols::image_copy_capture::ImageCopyCaptureState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1: ()
        ] => $crate::protocols::image_copy_capture::ImageCopyCaptureState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1: ()
        ] => $crate::protocols::image_copy_capture::ImageCopyCaptureState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1: std::sync::Mutex<$crate::protocols::image_copy_capture::CaptureFrameData>
        ] => $crate::protocols::image_copy_capture::ImageCopyCaptureState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::ext_image_copy_capture_cursor_session_v1::ExtImageCopyCaptureCursorSessionV1: ()
        ] => $crate::protocols::image_copy_capture::ImageCopyCaptureState);
    };
}

#[cfg(test)]
mod tests {
    use super::capture_too_soon;
    use std::time::Duration;

    #[test]
    fn uncapped_is_never_too_soon() {
        assert!(!capture_too_soon(
            Some(Duration::from_secs(1)),
            Duration::from_secs(1),
            None
        ));
    }

    #[test]
    fn first_frame_is_never_too_soon() {
        assert!(!capture_too_soon(
            None,
            Duration::from_secs(1),
            Some(Duration::from_millis(16))
        ));
    }

    #[test]
    fn within_interval_is_too_soon() {
        let iv = Some(Duration::from_millis(16));
        assert!(capture_too_soon(
            Some(Duration::from_millis(100)),
            Duration::from_millis(110),
            iv
        ));
    }

    #[test]
    fn at_or_past_interval_is_not_too_soon() {
        let iv = Some(Duration::from_millis(16));
        // Exactly at the boundary: elapsed == interval is not < interval.
        assert!(!capture_too_soon(
            Some(Duration::from_millis(100)),
            Duration::from_millis(116),
            iv
        ));
        assert!(!capture_too_soon(
            Some(Duration::from_millis(100)),
            Duration::from_millis(200),
            iv
        ));
    }
}
