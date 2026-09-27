//! `auto_navigate_on_click`: a completed click on a partially off-screen
//! window pans the camera to it (like activation), while a fully visible
//! window keeps focus-only. Content clicks pan immediately on release; only the
//! SSD title bar defers past the double-click window (its own double-click-fit
//! must beat the pan). Tests split at the resolve/fire seam: the content path
//! pans inside `resolve` (assert `camera_target`), the title-bar path schedules
//! a deferred fire (assert the timer), and `fire` performs the final gated pan
//! (assert `camera_target`).

use driftwm::config::{BTN_LEFT, BTN_RIGHT, Config};
use smithay::utils::{Point, SERIAL_COUNTER};

use super::{Fixture, map_window, window_by_app_id};

fn config_on() -> Config {
    Config::from_toml(
        r#"
        [navigation]
        auto_navigate_on_click = true
    "#,
    )
    .unwrap()
}

/// Slide the active output's camera far enough that `window` is no longer fully
/// in view, and clear any camera_target the move produced.
fn clip_window(f: &mut Fixture, window: &smithay::desktop::Window) {
    let cam = f.state().camera();
    f.state().set_camera(cam + Point::from((5000.0, 0.0)));
    f.state().with_output_state(|os| os.camera_target = None);
    assert!(!f.state().window_fully_in_viewport(window));
}

fn focus(f: &mut Fixture, window: &smithay::desktop::Window) {
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(window, serial);
}

/// Flag on + a clipped, focused window + a completed content click → the camera
/// pans immediately on release, without deferral.
#[test]
fn content_click_on_clipped_window_pans() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "clip", (640, 480));
    let window = window_by_app_id(&mut f, "clip").unwrap();
    clip_window(&mut f, &window);
    focus(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);

    let press = Point::from((10.0, 10.0));
    f.state()
        .arm_click_navigate(&window, press, BTN_LEFT, false);
    f.state().resolve_click_navigate(BTN_LEFT, press);

    assert!(
        f.state().camera_target().is_some(),
        "a completed content click should pan immediately"
    );
    assert!(
        f.state().click_navigate_timer.is_none(),
        "content clicks must not defer"
    );
}

/// Flag off + a clipped window + a completed click → nothing armed, nothing
/// pans (arm no-ops without the flag).
#[test]
fn click_does_not_pan_when_disabled() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "off", (640, 480));
    let window = window_by_app_id(&mut f, "off").unwrap();
    clip_window(&mut f, &window);

    let press = Point::from((10.0, 10.0));
    f.state()
        .arm_click_navigate(&window, press, BTN_LEFT, false);
    f.state().resolve_click_navigate(BTN_LEFT, press);

    assert!(f.state().pending_click_navigate.is_none());
    assert!(f.state().click_navigate_timer.is_none());
    assert!(
        f.state().camera_target().is_none(),
        "the flag is off, so a click must never pan"
    );
}

/// The pointer travels past the slop between press and release → treated as a
/// drag, no pan.
#[test]
fn drag_beyond_slop_does_not_pan() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "drag", (640, 480));
    let window = window_by_app_id(&mut f, "drag").unwrap();
    clip_window(&mut f, &window);
    focus(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);

    let press = Point::from((10.0, 10.0));
    f.state()
        .arm_click_navigate(&window, press, BTN_LEFT, false);
    // Release 100 px away (zoom 1 → 100 screen px, well past the 5 px slop).
    f.state()
        .resolve_click_navigate(BTN_LEFT, press + Point::from((100.0, 0.0)));

    assert!(
        f.state().camera_target().is_none(),
        "a drag past the slop must not pan"
    );
    assert!(f.state().click_navigate_timer.is_none());
}

/// A release of a different button keeps the pending armed (no pan); the armed
/// button's release then pans.
#[test]
fn button_mismatch_keeps_pending_until_armed_button_lifts() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "btn", (640, 480));
    let window = window_by_app_id(&mut f, "btn").unwrap();
    clip_window(&mut f, &window);
    focus(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);

    let press = Point::from((10.0, 10.0));
    f.state()
        .arm_click_navigate(&window, press, BTN_LEFT, false);

    // The right button lifts first (an already-held chord): keep waiting.
    f.state().resolve_click_navigate(BTN_RIGHT, press);
    assert!(
        f.state().pending_click_navigate.is_some(),
        "a mismatched button release must not consume the pending"
    );
    assert!(f.state().camera_target().is_none());

    // The armed button lifts: pan.
    f.state().resolve_click_navigate(BTN_LEFT, press);
    assert!(f.state().pending_click_navigate.is_none());
    assert!(
        f.state().camera_target().is_some(),
        "the armed button's release should pan"
    );
    assert!(f.state().click_navigate_timer.is_none());
}

/// Press on one output, release while a different output is active → the coords
/// are incompatible, so the pending is dropped without scheduling.
#[test]
fn release_on_different_output_drops_pending() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1280, 720));
    let id = f.add_client();
    map_window(&mut f, id, "xout", (640, 480));
    let window = window_by_app_id(&mut f, "xout").unwrap();

    // Arm against the default active output (HEADLESS-1).
    let press = Point::from((10.0, 10.0));
    f.state()
        .arm_click_navigate(&window, press, BTN_LEFT, false);

    // Switch the active output before the release resolves.
    f.state().focused_output = Some(out2);
    f.state().with_output_state(|os| os.camera_target = None);
    f.state().resolve_click_navigate(BTN_LEFT, press);

    assert!(f.state().pending_click_navigate.is_none());
    assert!(f.state().click_navigate_timer.is_none());
    assert!(
        f.state().camera_target().is_none(),
        "a cross-output release must not pan"
    );
}

/// The slop is screen-space: at zoom 0.5 a canvas travel of 8 px (4 screen px)
/// still schedules, while 12 px (6 screen px) does not.
#[test]
fn slop_is_screen_space_at_zoom() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "zoom", (640, 480));
    let window = window_by_app_id(&mut f, "zoom").unwrap();
    f.state().set_zoom(0.5);

    let press = Point::from((10.0, 10.0));
    f.state().arm_click_navigate(&window, press, BTN_LEFT, true);
    f.state()
        .resolve_click_navigate(BTN_LEFT, press + Point::from((8.0, 0.0)));
    assert!(
        f.state().click_navigate_timer.is_some(),
        "4 screen px is within the slop"
    );

    f.state().cancel_click_navigate();
    f.state().arm_click_navigate(&window, press, BTN_LEFT, true);
    f.state()
        .resolve_click_navigate(BTN_LEFT, press + Point::from((12.0, 0.0)));
    assert!(
        f.state().click_navigate_timer.is_none(),
        "6 screen px is past the slop"
    );
}

/// A title-bar click (`defer`) waits out the double-click window: resolve
/// schedules the deferred fire instead of panning now, so the title bar's own
/// double-click-fit can still cancel it. Same clipped+focused setup as the
/// content path — only `defer` differs.
#[test]
fn titlebar_click_defers_the_pan() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "bar", (640, 480));
    let window = window_by_app_id(&mut f, "bar").unwrap();
    clip_window(&mut f, &window);
    focus(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);

    let press = Point::from((10.0, 10.0));
    f.state().arm_click_navigate(&window, press, BTN_LEFT, true);
    f.state().resolve_click_navigate(BTN_LEFT, press);

    assert!(
        f.state().click_navigate_timer.is_some(),
        "a title-bar click should schedule the deferred navigate"
    );
    assert!(
        f.state().camera_target().is_none(),
        "the deferred path must not pan inside resolve"
    );
}

/// Fire on a clipped, focused window → the pan starts.
#[test]
fn fire_on_clipped_focused_window_navigates() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "clip", (640, 480));
    let window = window_by_app_id(&mut f, "clip").unwrap();
    clip_window(&mut f, &window);
    focus(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);

    f.state().fire_click_navigate(&window);

    assert!(
        f.state().camera_target().is_some(),
        "a clipped, focused window should pan when the deferred fire lands"
    );
}

/// Fire on a fully visible window → focus only, no pan (visibility gate).
#[test]
fn fire_on_visible_window_does_not_navigate() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "vis", (640, 480));
    let window = window_by_app_id(&mut f, "vis").unwrap();
    focus(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);
    assert!(f.state().window_fully_in_viewport(&window));

    f.state().fire_click_navigate(&window);

    assert!(
        f.state().camera_target().is_none(),
        "a fully visible window must stay put"
    );
}

/// Fire on a window whose client died during the delay → no pan, no panic.
#[test]
fn fire_on_dead_window_is_noop() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "dead", (640, 480));
    let window = window_by_app_id(&mut f, "dead").unwrap();
    clip_window(&mut f, &window);
    f.state().with_output_state(|os| os.camera_target = None);

    f.kill_client(id);
    f.pump(10);

    f.state().fire_click_navigate(&window);

    assert!(
        f.state().camera_target().is_none(),
        "a dead window must not pan"
    );
}

/// Focus moved to another window before the fire (e.g. Alt-Tab) → no pan; the
/// deferred fire must not yank the camera back.
#[test]
fn fire_after_focus_moved_does_not_navigate() {
    let mut f = Fixture::with_config(config_on());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "a", (640, 480));
    map_window(&mut f, id, "b", (640, 480));
    let win_a = window_by_app_id(&mut f, "a").unwrap();
    let win_b = window_by_app_id(&mut f, "b").unwrap();
    clip_window(&mut f, &win_a);

    // Focus moves to B during the delay.
    focus(&mut f, &win_b);
    f.state().with_output_state(|os| os.camera_target = None);

    f.state().fire_click_navigate(&win_a);

    assert!(
        f.state().camera_target().is_none(),
        "a window that lost focus during the delay must not pan"
    );
}
