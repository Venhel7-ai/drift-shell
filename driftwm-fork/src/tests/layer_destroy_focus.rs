//! Pointer-focus invariants when a layer appears or disappears under a
//! stationary cursor: `pointer_over_layer` is kept current by the hit test, so
//! a layer destroyed (or revealed by a fullscreen exit) beneath a resting
//! cursor would route the next press/scroll to the canvas instead of the layer
//! surface under the pointer. The per-iteration pull re-picks regardless of
//! whether it delivers, and the tests pin the press to the correct grab.

use driftwm::config::BTN_LEFT;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::input_backend::{pointer_to_screen, press, release};
use super::{
    Fixture, assert_click_grab, client::LayerConfigureProps, input_backend::FakeDevice,
    pointer_focus,
};

use smithay::utils::Point;

/// Server-side surface of the layer with `namespace` on the first output.
fn layer_surface_by_namespace(
    f: &mut Fixture,
    namespace: &str,
) -> smithay::reexports::wayland_server::protocol::wl_surface::WlSurface {
    let output = f.state().space.outputs().next().cloned().unwrap();
    smithay::desktop::layer_map_for_output(&output)
        .layers()
        .find(|l| l.namespace() == namespace)
        .unwrap()
        .wl_surface()
        .clone()
}

/// Map a layer, give it a buffer, and settle. Returns the client-side surface.
fn map_layer(
    f: &mut Fixture,
    id: super::client::ClientId,
    layer: zwlr_layer_shell_v1::Layer,
    namespace: &str,
    size: (u32, u32),
    anchor: zwlr_layer_surface_v1::Anchor,
) -> wayland_client::protocol::wl_surface::WlSurface {
    let created = f.client(id).create_layer(None, layer, namespace);
    let surface = created.surface.clone();
    created.set_configure_props(LayerConfigureProps {
        size: Some(size),
        anchor: Some(anchor),
        exclusive_zone: Some(0),
        ..Default::default()
    });
    created.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.set_size(size.0 as u16, size.1 as u16);
    layer.attach_new_buffer();
    layer.ack_last_and_commit();
    f.double_roundtrip(id);
    surface
}

/// A panel layer dies over a bar while the cursor rests on the bar. The next
/// press — no pointer motion through the whole teardown — must reach the bar
/// again, not pan the canvas.
#[test]
fn layer_destroyed_under_the_cursor_keeps_pointer_focus() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    // The bar: full-width bottom strip with no exclusive zone.
    let bar = map_layer(
        &mut f,
        id,
        zwlr_layer_shell_v1::Layer::Top,
        "bar",
        (1920, 40),
        zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );

    // A covering layer above the bar, over the cursor's spot: a panel or OSD
    // that opens anchored to the bar, and dies on the click that hits the bar.
    let panel = map_layer(
        &mut f,
        id,
        zwlr_layer_shell_v1::Layer::Overlay,
        "panel",
        (400, 600),
        zwlr_layer_surface_v1::Anchor::Bottom,
    );

    let over_bar = Point::from((960.0, 1060.0));
    pointer_to_screen(&mut f, &device, over_bar);
    let bar_server = layer_surface_by_namespace(&mut f, "bar");
    let panel_server = layer_surface_by_namespace(&mut f, "panel");
    assert_eq!(
        pointer_focus(&mut f),
        Some(panel_server),
        "the panel sits over the cursor's spot, so it owns focus while alive"
    );

    // The click that hits the bar closes the panel: layer-shell role first,
    // wl_surface after (the teardown order a real client uses).
    f.client(id).layer(&panel).layer_surface.destroy();
    f.double_roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(bar_server.clone()),
        "focus must be re-seated on the bar beneath, not left on the dead \
         panel and not dropped to the canvas"
    );

    // The next press, still without pointer motion, must reach the bar again —
    // not pan the canvas (unmodified BTN_LEFT on empty canvas is pan-viewport
    // in the default config). A press over a layer always installs a
    // ScreenSpaceClickGrab; a PanGrab is the stale-flag regression.
    press(&mut f, &device, BTN_LEFT);
    assert_click_grab(
        &mut f,
        "the press after the teardown must hit the bar, not pan the canvas",
    );
    release(&mut f, &device, BTN_LEFT);

    // Cleanup: the panel's wl_surface, then the bar's layer surface.
    f.client(id).layer(&panel).surface.destroy();
    f.client(id).layer(&bar).layer_surface.destroy();
    f.double_roundtrip(id);
}

/// Exiting fullscreen can restore a hidden bar beneath a stationary cursor.
/// The pull after the exit re-seats focus on the revealed bar.
#[test]
fn fullscreen_exit_reveals_a_bar_under_the_stationary_cursor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let window_surface = super::map_window(&mut f, id, "w", (800, 600));
    let output = f.state().active_output().unwrap();
    let window = super::window_by_app_id(&mut f, "w").unwrap();

    // The bar fullscreen hides and exit restores beneath the cursor.
    let bar = map_layer(
        &mut f,
        id,
        zwlr_layer_shell_v1::Layer::Top,
        "bar",
        (1920, 40),
        zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );

    // Cursor over the bar's strip, then fullscreen over it.
    let over_bar = Point::from((960.0, 1060.0));
    pointer_to_screen(&mut f, &device, over_bar);
    f.state().enter_fullscreen(&window, Some(output.clone()));
    f.double_roundtrip(id);
    super::adopt_last_configure(&mut f, id, &window_surface);

    // The fullscreen window owns focus while it covers the bar.
    assert_eq!(
        pointer_focus(&mut f),
        Some(super::server_surface(&window)),
        "the fullscreen window owns focus while the bar is hidden beneath it"
    );

    // Exit with the cursor still over the bar's spot, no motion since.
    f.state().exit_fullscreen_on(&output);
    f.double_roundtrip(id);

    let bar_server = layer_surface_by_namespace(&mut f, "bar");
    assert_eq!(
        pointer_focus(&mut f),
        Some(bar_server),
        "exiting fullscreen must re-seat focus on the bar revealed beneath the cursor"
    );

    press(&mut f, &device, BTN_LEFT);
    assert_click_grab(
        &mut f,
        "the press must hit the bar, not the restored window",
    );
    release(&mut f, &device, BTN_LEFT);

    // Cleanup: the window, then the bar's layer surface.
    f.client(id).window(&window_surface).destroy();
    f.client(id).layer(&bar).layer_surface.destroy();
    f.double_roundtrip(id);
}

/// The same reveal, with the cursor also inside the rect the exiting window is
/// heading for. That engages the pull's transition hold, which keeps the
/// window's delivery through the client's resize; it must not also keep the
/// stale routing flag, or the press and scroll that follow go to the window
/// behind the bar instead of the bar.
#[test]
fn fullscreen_exit_over_a_bar_inside_the_restored_rect_still_routes_to_the_bar() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let bar_id = f.add_client();
    let device = FakeDevice::mouse();

    let window_surface = super::map_window(&mut f, id, "w", (800, 600));
    let output = f.state().active_output().unwrap();
    let window = super::window_by_app_id(&mut f, "w").unwrap();
    // The fixture camera starts at (-960, -540), so a window at the canvas
    // origin spans screen (960, 540)..(1760, 1140): its bottom edge runs
    // under a bottom-anchored bar.
    f.state().map_window(
        crate::state::StageWindow::Client(window.clone()),
        Point::from((0, 0)),
        false,
    );

    let bar = map_layer(
        &mut f,
        bar_id,
        zwlr_layer_shell_v1::Layer::Top,
        "bar",
        (1920, 40),
        zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );

    // Over the bar's strip *and* inside the window's rect.
    let cursor = Point::from((1200.0, 1060.0));
    pointer_to_screen(&mut f, &device, cursor);
    let bar_server = layer_surface_by_namespace(&mut f, "bar");
    assert_eq!(
        pointer_focus(&mut f),
        Some(bar_server.clone()),
        "the bar must sit over the window at the cursor's spot, or the exit \
         reveals nothing"
    );

    f.state().enter_fullscreen(&window, Some(output.clone()));
    f.double_roundtrip(id);
    super::adopt_last_configure(&mut f, id, &window_surface);
    assert_eq!(
        pointer_focus(&mut f),
        Some(super::server_surface(&window)),
        "the fullscreen window owns focus while the bar is hidden beneath it"
    );

    // Do not answer the resize: with the fullscreen-sized buffer still held,
    // the recentre stays owed and the hold engages on every pull.
    f.state().exit_fullscreen_on(&output);
    f.double_roundtrip(id);
    assert!(
        !f.state().pending_recenter.is_empty(),
        "the exit must leave a recentre owed, or the hold never engages and \
         this scenario tests the plain reveal"
    );

    let bar_buttons_before = f.client(bar_id).state.pointer_buttons.len();
    let window_buttons_before = f.client(id).state.pointer_buttons.len();
    press(&mut f, &device, BTN_LEFT);
    assert_click_grab(
        &mut f,
        "the press must hit the revealed bar, not pan the canvas or raise the \
         window behind it",
    );
    release(&mut f, &device, BTN_LEFT);
    f.double_roundtrip(bar_id);
    f.double_roundtrip(id);
    assert!(
        f.client(bar_id).state.pointer_buttons.len() > bar_buttons_before,
        "the bar's client must see the press"
    );
    assert_eq!(
        f.client(id).state.pointer_buttons.len(),
        window_buttons_before,
        "the window behind the bar must not see the press"
    );

    let camera_before = f.state().camera();
    let bar_axes_before = f.client(bar_id).state.pointer_axes.len();
    super::input_backend::trackpad_scroll(&mut f, &device);
    f.double_roundtrip(bar_id);
    assert_eq!(
        f.state().camera(),
        camera_before,
        "a scroll over the bar must reach the bar, not pan the canvas"
    );
    assert!(
        f.client(bar_id).state.pointer_axes.len() > bar_axes_before,
        "the bar's client must see the scroll"
    );

    f.client(id).window(&window_surface).destroy();
    f.client(bar_id).layer(&bar).layer_surface.destroy();
    f.double_roundtrip(id);
    f.double_roundtrip(bar_id);
}

/// The reveal and the press in one dispatch: no pull runs between the exit
/// and the finger landing, so the press has to make focus current itself or
/// it routes on the previous iteration's answer — the window, not the bar.
#[test]
fn a_press_in_the_same_dispatch_as_the_reveal_still_reaches_the_bar() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let bar_id = f.add_client();
    let device = FakeDevice::mouse();

    let window_surface = super::map_window(&mut f, id, "w", (800, 600));
    let output = f.state().active_output().unwrap();
    let window = super::window_by_app_id(&mut f, "w").unwrap();

    let bar = map_layer(
        &mut f,
        bar_id,
        zwlr_layer_shell_v1::Layer::Top,
        "bar",
        (1920, 40),
        zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );

    let over_bar = Point::from((960.0, 1060.0));
    pointer_to_screen(&mut f, &device, over_bar);
    f.state().enter_fullscreen(&window, Some(output.clone()));
    f.double_roundtrip(id);
    super::adopt_last_configure(&mut f, id, &window_surface);
    assert_eq!(
        pointer_focus(&mut f),
        Some(super::server_surface(&window)),
        "the fullscreen window owns focus while the bar is hidden beneath it"
    );

    // No roundtrip between these two: the exit and the press share a dispatch.
    f.state().exit_fullscreen_on(&output);
    press(&mut f, &device, BTN_LEFT);
    assert_click_grab(
        &mut f,
        "a press landing in the same dispatch as the reveal must still reach the bar",
    );
    release(&mut f, &device, BTN_LEFT);

    f.client(id).window(&window_surface).destroy();
    f.client(bar_id).layer(&bar).layer_surface.destroy();
    f.double_roundtrip(id);
    f.double_roundtrip(bar_id);
}
