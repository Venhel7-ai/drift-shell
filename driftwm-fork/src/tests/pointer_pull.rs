//! `DriftWm::refresh_pointer_focus` runs once per iteration from
//! `refresh_and_flush_clients`, so every fixture pump is a pull. Coverage
//! scenarios: a popup or layer mapping under a stationary cursor, a
//! compositor-side move shoving a neighbour or nudging a window. Contract
//! scenarios pin the rules that keep a per-iteration recompute honest: never
//! dispatch through a grab that supplies its own focus (a live popup grab
//! excepted), never re-seat a lock whose target is unchanged, and re-evaluate
//! a constraint on every pull even when nothing is delivered. Design notes in
//! `dev/docs/caveats.md`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay::desktop::Window;
use smithay::input::SeatHandler;
use smithay::input::pointer::{
    ButtonEvent, Focus, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1;

use driftwm::config::{Action, BTN_LEFT, Direction};

use crate::input::PointerGrabKind;
use crate::ipc::dispatch;
use crate::ipc::protocol::{Request, Response, WindowSelector};
use crate::state::{DriftWm, StageWindow};

use super::client::ClientId;
use super::input_backend::{
    FakeDevice, key_press, key_release, pen_proximity_in, pen_to, pointer_to, pointer_to_screen,
    press, release, tablet_added,
};
use super::{
    Fixture, assert_click_grab, end_grab, first_popup_surface, keyboard_focus, map_popup,
    map_top_layer, map_window, pointer_focus, server_surface, window_by_app_id,
};

/// Map a parent window, park a stationary cursor where the default
/// positioner's popup will land (centred on the parent's top-left corner),
/// take a real press+release over the parent for a serial, then open a popup
/// there and grab it. Returns the parent window, its stage position, the
/// cursor, and the popup's client surface.
fn setup_grabbed_popup_over_stationary_cursor(
    f: &mut Fixture,
    id: ClientId,
    device: &FakeDevice,
) -> (
    Window,
    Point<f64, Logical>,
    Point<f64, Logical>,
    wayland_client::protocol::wl_surface::WlSurface,
) {
    let parent = map_window(f, id, "parent", (400, 300));
    let parent_window = window_by_app_id(f, "parent").unwrap();
    let parent_pos = f
        .state()
        .stage
        .position_of(&parent_window)
        .unwrap()
        .to_f64();

    let cursor = parent_pos + Point::from((10.0, 10.0));
    pointer_to(f, device, cursor);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(f),
        Some(server_surface(&parent_window)),
        "test setup bug: the cursor must start over the parent"
    );

    press(f, device, BTN_LEFT);
    release(f, device, BTN_LEFT);
    f.double_roundtrip(id);

    let popup = f.client(id).create_popup(&parent);
    let popup_surface = popup.surface.clone();
    popup.commit();
    f.roundtrip(id);
    let popup = f.client(id).popup(&popup_surface);
    popup.grab(1);
    popup.attach_new_buffer();
    popup.ack_last_and_commit();
    f.double_roundtrip(id);

    // A freshly mapped popup's input region is a single logical pixel until
    // its viewport destination is set: grow it to the positioner's own size,
    // which the default centres on the cursor above.
    super::popups::grow_popup(f, id, &popup_surface, (200, 100));

    let popup_server = first_popup_surface(&server_surface(&parent_window)).unwrap();
    assert_eq!(
        pointer_focus(f),
        Some(popup_server),
        "test setup bug: the popup must cover the stationary cursor and take \
         pointer focus"
    );
    assert!(
        !f.client(id).state.pointer_positions.is_empty(),
        "test setup bug: the popup's client must have been handed a position"
    );

    (parent_window, parent_pos, cursor, popup_surface)
}

/// A popup that takes a popup grab, mapping under a stationary cursor, takes
/// pointer focus; dismissing it hands focus back to the parent beneath the
/// cursor.
#[test]
fn a_grabbed_popup_mapping_under_a_stationary_cursor_takes_pointer_focus_and_hands_it_back() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let (parent_window, parent_pos, cursor, popup_surface) =
        setup_grabbed_popup_over_stationary_cursor(&mut f, id, &device);

    f.client(id).popup(&popup_surface).destroy();
    // The teardown defers the pointer ungrab to an idle: the destroy's own
    // pull tears the dead grab down, and the idle's restore motion lands on
    // the pull after that.
    f.double_roundtrip(id);
    f.pump(3);

    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&parent_window)),
        "dismissing the popup must return pointer focus to the parent \
         beneath the stationary cursor"
    );
    let expected_local = cursor - parent_pos;
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(expected_local.x, expected_local.y)),
        "the parent must be handed a motion carrying its own local point"
    );
}

/// evdev code for a key with no default binding — the space `key_press`
/// reports in.
const KEY_Z: u32 = 44;

/// Dismissing a grabbing popup hands keyboard focus back to its parent. A
/// popup grab seats keyboard focus on the popup lazily, at the first key it
/// forwards, so the test presses one first; tearing the dead grab down
/// restores nothing on its own, and the pull has to re-derive focus.
#[test]
fn dismissing_a_grabbing_popup_hands_keyboard_focus_back_to_its_parent() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let (parent_window, _parent_pos, _cursor, popup_surface) =
        setup_grabbed_popup_over_stationary_cursor(&mut f, id, &device);

    key_press(&mut f, KEY_Z);
    key_release(&mut f, KEY_Z);
    let popup_server = first_popup_surface(&server_surface(&parent_window)).unwrap();
    assert_eq!(
        keyboard_focus(&mut f),
        Some(popup_server),
        "test setup bug: a key event through the grab must seat keyboard \
         focus on the popup"
    );

    f.client(id).popup(&popup_surface).destroy();
    // The ungrab idle fires after the destroy's own pull; see above.
    f.double_roundtrip(id);
    f.pump(3);

    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&parent_window)),
        "dismissing the grabbing popup must hand keyboard focus back to its \
         parent"
    );
}

/// Same shape as the grabbed popup above, but a grab-less popup (a tooltip):
/// no grab teardown or idle to wait out.
#[test]
fn a_grabless_popup_mapping_under_a_stationary_cursor_takes_focus_and_hands_it_back() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let parent = map_window(&mut f, id, "parent", (400, 300));
    let parent_window = window_by_app_id(&mut f, "parent").unwrap();
    let parent_pos = f
        .state()
        .stage
        .position_of(&parent_window)
        .unwrap()
        .to_f64();

    let cursor = parent_pos + Point::from((10.0, 10.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&parent_window)),
        "test setup bug: the cursor must start over the parent"
    );

    let popup_surface = map_popup(&mut f, id, &parent);
    // See the grabbed-popup helper: a freshly mapped popup is a single
    // logical pixel until its viewport destination is set.
    super::popups::grow_popup(&mut f, id, &popup_surface, (200, 100));
    let popup_server = first_popup_surface(&server_surface(&parent_window)).unwrap();
    assert_eq!(
        pointer_focus(&mut f),
        Some(popup_server),
        "the grab-less popup covering the stationary cursor must still take \
         pointer focus"
    );
    assert!(
        !f.client(id).state.pointer_positions.is_empty(),
        "the popup's client must have been handed a position"
    );

    f.client(id).popup(&popup_surface).destroy();
    f.double_roundtrip(id);

    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&parent_window)),
        "unmapping the popup must hand pointer focus back to the parent \
         beneath the stationary cursor"
    );
    let expected_local = cursor - parent_pos;
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(expected_local.x, expected_local.y)),
        "the parent must be handed a motion carrying its own local point"
    );
}

/// `fit_window_snapped` on window A shoves its snapped neighbour B. A
/// stationary cursor resting well inside B — far enough from its near edge
/// that the shove cannot push it past — must be delivered B's new local
/// point.
#[test]
fn fit_window_snapped_shoves_a_stationary_cursor_resting_on_the_snapped_neighbour() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, id, "a", (400, 300));
    let a = window_by_app_id(&mut f, "a").unwrap();
    f.state().map_window(
        StageWindow::Client(a.clone()),
        Point::from((100, 100)),
        false,
    );

    // B is far wider than any shove `fit_window_snapped` can cause (bounded by
    // the 1920px-wide usable area), so a cursor parked deep inside it stays
    // covered whichever way the shift lands.
    map_window(&mut f, id, "b", (3000, 300));
    let b = window_by_app_id(&mut f, "b").unwrap();
    let a_frame = f
        .state()
        .visual_frame_rect(&StageWindow::Client(a.clone()))
        .unwrap();
    let gap = f.state().config.snap_gap as i32;
    let bw = f.state().default_border_width();
    let b_loc = Point::from((a_frame.x_high as i32 + gap + bw, 100));
    f.state()
        .map_window(StageWindow::Client(b.clone()), b_loc, false);

    f.state().with_output_state(|os| {
        os.zoom = 1.0;
        os.camera = Point::from((0.0, 0.0));
    });

    let cursor = Point::from((f64::from(b_loc.x) + 900.0, 250.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&b)),
        "the cursor must start over B, or this scenario tests nothing"
    );

    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);
    f.roundtrip(id);

    let positions_before = f.client(id).state.pointer_positions.len();
    let frames_before = f.client(id).state.pointer_frames;
    f.state().execute_action(&Action::FitWindowSnapped);
    f.roundtrip(id);

    let b_pos_after = f.state().stage.position_of(&b).unwrap();
    assert_ne!(
        b_pos_after, b_loc,
        "test setup bug: the fit must actually shove B, or this scenario \
         tests nothing"
    );
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&b)),
        "the cursor must still be over B after the shove, or this scenario \
         does not exercise the case it is about"
    );

    let expected_local = cursor - b_pos_after.to_f64();
    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before + 1,
        "exactly one new motion, carrying the point the shove actually moved \
         B to"
    );
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(expected_local.x, expected_local.y))
    );
    assert_eq!(
        f.client(id).state.pointer_frames,
        frames_before + 1,
        "...paired with exactly one frame"
    );
}

/// `Action::NudgeWindow` on the window under a stationary cursor delivers the
/// new local point: the old one minus the nudge delta.
#[test]
fn nudge_window_delivers_the_new_local_point_to_a_stationary_cursor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, id, "w", (400, 300));
    let window = window_by_app_id(&mut f, "w").unwrap();
    let pos = f.state().stage.position_of(&window).unwrap().to_f64();
    let cursor = pos + Point::from((150.0, 120.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&window)),
        "the cursor must start over the window, or this scenario tests nothing"
    );

    let positions_before = f.client(id).state.pointer_positions.len();
    let frames_before = f.client(id).state.pointer_frames;
    f.state()
        .execute_action(&Action::NudgeWindow(Direction::Right));
    f.roundtrip(id);

    let step = f64::from(f.state().config.nudge_step);
    let new_pos = f.state().stage.position_of(&window).unwrap().to_f64();
    assert_eq!(
        new_pos,
        pos + Point::from((step, 0.0)),
        "test setup bug: the nudge must actually move the window right by its step"
    );

    let expected_local = cursor - new_pos;
    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before + 1,
        "exactly one new motion"
    );
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(expected_local.x, expected_local.y)),
        "carrying the old local point minus the nudge delta"
    );
    assert_eq!(f.client(id).state.pointer_frames, frames_before + 1);
}

/// The same class of compositor-side move, through the IPC `move` verb rather
/// than a keybinding.
#[test]
fn ipc_move_delivers_the_new_local_point_to_a_stationary_cursor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, id, "w", (400, 300));
    let window = window_by_app_id(&mut f, "w").unwrap();
    let pos = f.state().stage.position_of(&window).unwrap().to_f64();
    let cursor = pos + Point::from((100.0, 80.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(id);

    let Ok(Response::Position { x, y }) = dispatch(
        Request::Move {
            window: None,
            to: None,
        },
        f.state(),
    ) else {
        panic!("test setup bug: reading the position must succeed");
    };

    let positions_before = f.client(id).state.pointer_positions.len();
    dispatch(
        Request::Move {
            window: None,
            to: Some((x + 100, y)),
        },
        f.state(),
    )
    .unwrap();
    f.roundtrip(id);

    let new_pos = f.state().stage.position_of(&window).unwrap().to_f64();
    assert_ne!(
        new_pos, pos,
        "test setup bug: the move must actually relocate the window"
    );
    let expected_local = cursor - new_pos;
    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before + 1,
        "the compositor-side move must deliver exactly one new motion"
    );
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(expected_local.x, expected_local.y)),
        "carrying the point the move actually left under the stationary cursor"
    );
}

/// A pen in proximity resting on a window that is nudged gets the new local
/// point on the seat's `wl_pointer` — the pull refreshes the seat pointer,
/// not tablet-v2 tool motion.
#[test]
fn a_pen_resting_on_a_nudged_window_gets_the_new_local_point() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, "w", (400, 300));
    let window = window_by_app_id(&mut f, "w").unwrap();
    let pos = f.state().stage.position_of(&window).unwrap().to_f64();
    let at = pos + Point::from((200.0, 150.0));

    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);
    pen_proximity_in(&mut f, &device, at);
    pen_to(&mut f, &device, at);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&window)),
        "the pen must start over the window, or this scenario tests nothing"
    );

    let positions_before = f.client(id).state.pointer_positions.len();
    f.state()
        .execute_action(&Action::NudgeWindow(Direction::Right));
    f.roundtrip(id);

    let new_pos = f.state().stage.position_of(&window).unwrap().to_f64();
    let expected_local = at - new_pos;
    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before + 1,
        "exactly one new wl_pointer position"
    );
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(expected_local.x, expected_local.y))
    );
    assert_eq!(
        f.state()
            .last_pointer_delivery
            .as_ref()
            .map(|(_, _, local)| *local),
        Some(expected_local),
        "last_pointer_delivery must record it too"
    );
}

/// A layer surface mapping under a stationary cursor gets `enter` with no
/// pointer motion in between, and a press on it then installs a
/// `ScreenSpaceClickGrab`.
#[test]
fn a_layer_surface_mapping_under_a_stationary_cursor_gets_enter_and_a_click_grab() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let over_bar = Point::from((960.0, 1060.0));
    pointer_to_screen(&mut f, &device, over_bar);
    f.roundtrip(id);
    assert!(
        pointer_focus(&mut f).is_none(),
        "test setup bug: the cursor must start over bare canvas"
    );

    let positions_before = f.client(id).state.pointer_positions.len();
    let bar = map_top_layer(
        &mut f,
        id,
        "bar",
        (1920, 40),
        Some(
            zwlr_layer_surface_v1::Anchor::Bottom
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
        ),
    );

    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before + 1,
        "exactly one enter for the stationary cursor now over the freshly \
         mapped bar"
    );
    // 1040 is the bottom-anchored 40 px bar's top edge on the 1080 px output.
    assert_eq!(
        f.client(id).state.pointer_positions.last(),
        Some(&(over_bar.x, over_bar.y - 1040.0))
    );

    press(&mut f, &device, BTN_LEFT);
    assert_click_grab(
        &mut f,
        "a press over the freshly mapped bar must install a ScreenSpaceClickGrab",
    );
    release(&mut f, &device, BTN_LEFT);

    f.client(id).layer(&bar).layer_surface.destroy();
    f.client(id).layer(&bar).surface.destroy();
    f.double_roundtrip(id);
}

/// A `PointerGrab` that counts every `motion` it receives, standing in for
/// any grab that supplies its own focus and does work in `motion`.
struct CountingGrab {
    start_data: GrabStartData<DriftWm>,
    motions: Arc<AtomicUsize>,
}

impl PointerGrab<DriftWm> for CountingGrab {
    fn motion(
        &mut self,
        data: &mut DriftWm,
        handle: &mut PointerInnerHandle<'_, DriftWm>,
        _focus: Option<(<DriftWm as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        self.motions.fetch_add(1, Ordering::SeqCst);
        handle.motion(data, None, event);
    }

    fn button(
        &mut self,
        data: &mut DriftWm,
        handle: &mut PointerInnerHandle<'_, DriftWm>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        handle.unset_grab(self, data, event.serial, event.time, true);
    }

    fn unset(&mut self, _data: &mut DriftWm) {}

    crate::grabs::forward_pointer_grab_methods!();
}

/// A live pointer grab is never ticked by the pull, even when a scene change
/// under it would otherwise force a dispatch.
#[test]
fn a_live_grab_is_never_ticked_by_the_pull() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, id, "w", (400, 300));
    let window = window_by_app_id(&mut f, "w").unwrap();
    f.state().map_window(
        StageWindow::Client(window.clone()),
        Point::from((0, 0)),
        false,
    );
    f.state().with_output_state(|os| {
        os.zoom = 1.0;
        os.camera = Point::from((0.0, 0.0));
    });

    let cursor = Point::from((100.0, 100.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&window)),
        "test setup bug: the cursor must start over the window"
    );

    let pointer = f.state().seat.get_pointer().unwrap();
    let motions = Arc::new(AtomicUsize::new(0));
    let serial = SERIAL_COUNTER.next_serial();
    pointer.set_grab(
        f.state(),
        CountingGrab {
            start_data: GrabStartData {
                focus: None,
                button: 0,
                location: cursor,
            },
            motions: motions.clone(),
        },
        serial,
        Focus::Keep,
    );
    assert!(
        f.state().seat.get_pointer().unwrap().is_grabbed(),
        "test setup bug: the grab must be installed"
    );

    // A scene change under the grab — a compositor-side move — that would
    // force a dispatch were the grab not skipped outright.
    f.state().map_window(
        StageWindow::Client(window.clone()),
        Point::from((500, 500)),
        false,
    );

    f.pump(5);

    assert_eq!(
        motions.load(Ordering::SeqCst),
        0,
        "a live grab must never be ticked by the pull, even though the \
         window moved under it"
    );

    end_grab(&mut f);
}

/// An open menu (a live popup grab) over a stationary cursor gets exactly one
/// motion over several pulls — not one per pull for as long as it stays open.
#[test]
fn an_open_menu_over_a_stationary_cursor_gets_no_further_motion_over_several_pulls() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let (_parent_window, _parent_pos, _cursor, popup_surface) =
        setup_grabbed_popup_over_stationary_cursor(&mut f, id, &device);

    let positions_before = f.client(id).state.pointer_positions.len();
    // Roundtrip rather than a bare pump: the client has to actually dispatch
    // for a wrongly-sent motion to show up here at all.
    for _ in 0..5 {
        f.roundtrip(id);
    }
    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before,
        "an open menu over a stationary cursor must get no further motion \
         over several pulls"
    );

    f.client(id).popup(&popup_surface).destroy();
    f.double_roundtrip(id);
    f.pump(3);
}

/// A menu open on one client, the cursor resting on another client's window,
/// and that window moved away under the stationary cursor. Dismissing the
/// menu must not hand the bystander an enter.
#[test]
fn a_window_that_left_a_stationary_cursor_under_an_open_menu_is_not_re_entered_on_dismiss() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let menu_owner = f.add_client();
    let bystander = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, bystander, "bystander", (400, 300));
    let bystander_window = window_by_app_id(&mut f, "bystander").unwrap();
    // The fixture cascades a second window over the first by a few pixels, so
    // park the bystander (by its centre, as IPC moves go) clear of where the
    // menu owner and its popup land, still inside the viewport.
    let bystander_centre = (700, 0);
    dispatch(
        Request::Move {
            window: Some(WindowSelector::AppId("bystander".into())),
            to: Some(bystander_centre),
        },
        f.state(),
    )
    .unwrap();
    f.roundtrip(bystander);
    let bystander_pos = f
        .state()
        .stage
        .position_of(&bystander_window)
        .unwrap()
        .to_f64();
    let (_parent_window, _parent_pos, _cursor, popup_surface) =
        setup_grabbed_popup_over_stationary_cursor(&mut f, menu_owner, &device);

    let cursor = bystander_pos + Point::from((50.0, 50.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(bystander);
    assert_eq!(
        f.state().surface_under(cursor, None).map(|(t, _)| t.0),
        Some(server_surface(&bystander_window)),
        "test setup bug: the cursor must rest on the bystander"
    );
    assert!(
        pointer_focus(&mut f).is_none(),
        "test setup bug: under the menu's grab the bystander must get no focus"
    );
    let positions_before = f.client(bystander).state.pointer_positions.len();

    dispatch(
        Request::Move {
            window: Some(WindowSelector::AppId("bystander".into())),
            to: Some((bystander_centre.0 + 2000, bystander_centre.1)),
        },
        f.state(),
    )
    .unwrap();
    f.roundtrip(bystander);

    f.client(menu_owner).popup(&popup_surface).destroy();
    f.double_roundtrip(menu_owner);
    f.pump(3);
    f.roundtrip(bystander);

    assert!(
        pointer_focus(&mut f).is_none(),
        "the cursor rests on bare canvas once the menu is gone"
    );
    assert_eq!(
        f.client(bystander).state.pointer_positions.len(),
        positions_before,
        "dismissing the menu must not re-enter a window that already left \
         the stationary cursor"
    );
}

/// As above, with the bystander's move and the menu's dismiss landing in the
/// same dispatch: the pull that finds the grab ended is the first to see the
/// bystander gone, and the restore must still not enter it.
#[test]
fn a_window_that_leaves_in_the_dismissing_dispatch_is_not_re_entered_either() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let menu_owner = f.add_client();
    let bystander = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, bystander, "bystander", (400, 300));
    let bystander_window = window_by_app_id(&mut f, "bystander").unwrap();
    let bystander_centre = (700, 0);
    dispatch(
        Request::Move {
            window: Some(WindowSelector::AppId("bystander".into())),
            to: Some(bystander_centre),
        },
        f.state(),
    )
    .unwrap();
    f.roundtrip(bystander);
    let bystander_pos = f
        .state()
        .stage
        .position_of(&bystander_window)
        .unwrap()
        .to_f64();
    let (_parent_window, _parent_pos, _cursor, popup_surface) =
        setup_grabbed_popup_over_stationary_cursor(&mut f, menu_owner, &device);

    let cursor = bystander_pos + Point::from((50.0, 50.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(bystander);
    assert_eq!(
        f.state().surface_under(cursor, None).map(|(t, _)| t.0),
        Some(server_surface(&bystander_window)),
        "test setup bug: the cursor must rest on the bystander"
    );
    let positions_before = f.client(bystander).state.pointer_positions.len();

    // Queued client-side until the roundtrip below; the move lands first on
    // the server, so one dispatch processes both.
    f.client(menu_owner).popup(&popup_surface).destroy();
    dispatch(
        Request::Move {
            window: Some(WindowSelector::AppId("bystander".into())),
            to: Some((bystander_centre.0 + 2000, bystander_centre.1)),
        },
        f.state(),
    )
    .unwrap();
    f.double_roundtrip(menu_owner);
    f.pump(3);
    f.roundtrip(bystander);

    assert!(
        pointer_focus(&mut f).is_none(),
        "the cursor rests on bare canvas once the menu is gone"
    );
    assert_eq!(
        f.client(bystander).state.pointer_positions.len(),
        positions_before,
        "a dismiss in the same dispatch as the bystander's departure must \
         not re-enter it"
    );
}

/// A persistent confine whose region is re-set around the parked cursor arms
/// on the next pull with no motion delivered.
#[test]
fn a_persistent_confine_arms_once_its_region_is_reset_around_the_parked_cursor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    let surface = map_window(&mut f, id, "w", (800, 600));
    let window = window_by_app_id(&mut f, "w").unwrap();
    let pos = f.state().stage.position_of(&window).unwrap().to_f64();

    let cursor = pos + Point::from((10.0, 10.0));
    pointer_to(&mut f, &device, cursor);
    f.roundtrip(id);

    let confine = f
        .client(id)
        .confine_pointer_with_region(&surface, &[(700, 500, 50, 50)]);
    f.double_roundtrip(id);
    assert!(
        !f.state().pointer_constraint_active(),
        "test setup bug: the region must exclude the parked cursor"
    );

    let positions_before = f.client(id).state.pointer_positions.len();

    f.client(id).set_confine_region(&confine, &[(0, 0, 50, 50)]);
    f.client(id).window(&surface).commit();
    f.roundtrip(id);

    assert!(
        f.state().pointer_constraint_active(),
        "the confine must arm once its region covers the parked cursor"
    );
    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_before,
        "arming a confine around an unmoved cursor must deliver no new motion"
    );
}

/// A click-drag that stays over the window it started on: smithay's implicit
/// `ClickGrab` keeps delivering to that window at `under`'s origin, so the
/// delivery record must already match what the client was handed — releasing
/// leaves nothing new to send.
#[test]
fn a_drag_inside_a_window_leaves_nothing_for_the_pull_to_send_at_release() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    map_window(&mut f, id, "w", (400, 300));
    let window = window_by_app_id(&mut f, "w").unwrap();
    f.state().map_window(
        StageWindow::Client(window.clone()),
        Point::from((0, 0)),
        false,
    );
    f.state().with_output_state(|os| {
        os.zoom = 1.0;
        os.camera = Point::from((0.0, 0.0));
    });

    let start = Point::from((100.0, 100.0));
    pointer_to_screen(&mut f, &device, start);
    f.roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&window)),
        "test setup bug: the cursor must start over the window"
    );

    press(&mut f, &device, BTN_LEFT);
    let pointer = f.state().seat.get_pointer().unwrap();
    assert_eq!(
        f.state().pointer_grab_kind(&pointer),
        PointerGrabKind::Click,
        "test setup bug: a plain press over window content must install \
         smithay's implicit click grab"
    );

    // The client has received the enter and both of these.
    pointer_to_screen(&mut f, &device, start + Point::from((5.0, 0.0)));
    pointer_to_screen(&mut f, &device, start + Point::from((10.0, 0.0)));
    f.roundtrip(id);
    let positions_after_drag = f.client(id).state.pointer_positions.len();

    release(&mut f, &device, BTN_LEFT);
    f.roundtrip(id);
    f.roundtrip(id);

    assert_eq!(
        f.client(id).state.pointer_positions.len(),
        positions_after_drag,
        "the pull must have nothing left to send once the click grab ends \
         over the same window it started on"
    );
}
