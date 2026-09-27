//! Per-output hot corners (`[outputs.hot_corners]`), dispatched directly
//! through `DriftWm::check_hot_corners` with an output and an output-local
//! screen position — exactly like the pointer-motion call sites do, minus the
//! motion-event plumbing. Covers config lookup (exact/wildcard/no-merge), the
//! square activation zone, the per-output entry latch (including its single
//! cross-output slot), and the fullscreen/dragging suppression rules.
//! `advance_hot_corner_latch` itself already has direct unit tests in
//! `src/input/mod.rs` (`hot_corner_tests`).

use smithay::output::Output;
use smithay::utils::{Logical, Point};

use driftwm::config::BTN_LEFT;

use super::{Fixture, client::ClientId, config, map_window};

const OUT_W: f64 = 1920.0;
const OUT_H: f64 = 1080.0;

fn top_left() -> Point<f64, Logical> {
    Point::from((2.0, 2.0))
}
fn top_right() -> Point<f64, Logical> {
    Point::from((OUT_W - 2.0, 2.0))
}
fn bottom_left() -> Point<f64, Logical> {
    Point::from((2.0, OUT_H - 2.0))
}
fn bottom_right() -> Point<f64, Logical> {
    Point::from((OUT_W - 2.0, OUT_H - 2.0))
}
fn center() -> Point<f64, Logical> {
    Point::from((OUT_W / 2.0, OUT_H / 2.0))
}

/// A zoom action sets `zoom_target` synchronously (the animation only lerps
/// `zoom` toward it later), so it's a reliable, per-output "did this fire" probe.
fn fired(output: &Output) -> bool {
    crate::state::output_state(output).zoom_target.is_some()
}

fn reset(output: &Output) {
    crate::state::output_state(output).zoom_target = None;
}

/// Map a toplevel, request fullscreen, and settle — leaves it fullscreen on
/// whichever output the compositor picks (the sole output in every caller here).
fn map_fullscreen_window(
    f: &mut Fixture,
    id: ClientId,
) -> wayland_client::protocol::wl_surface::WlSurface {
    let surface = map_window(f, id, "fs", (800, 600));
    f.client(id).window(&surface).set_fullscreen(None);
    f.double_roundtrip(id);
    f.client(id).window(&surface).ack_last_and_commit();
    f.double_roundtrip(id);
    surface
}

/// An exact `[[outputs]]` entry's own corners fire; a `name = "*"` entry's
/// corners cover an output with no exact entry, but never merge into one that
/// has an exact entry (own bindings only).
#[test]
fn exact_entry_wins_and_wildcard_covers_the_rest() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"

        [[outputs]]
        name = "*"
        [outputs.hot_corners]
        top_right = "zoom-out"
        "#,
    ));
    let out1 = f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1920, 1080));

    f.state().check_hot_corners(&out1, top_left());
    assert!(fired(&out1), "the exact entry's own corner fires");
    reset(&out1);

    f.state().check_hot_corners(&out1, center());
    f.state().check_hot_corners(&out1, top_right());
    assert!(
        !fired(&out1),
        "an exact entry does not also inherit the wildcard's corners"
    );

    // Real motion handlers set `focused_output` to the output they're
    // dispatching for before calling `check_hot_corners`; the actions it fires
    // read/write the active output's viewport state, so mirror that here.
    f.state().focused_output = Some(out2.clone());
    f.state().check_hot_corners(&out2, top_right());
    assert!(
        fired(&out2),
        "the wildcard covers an output with no exact entry"
    );
}

/// An exact entry with no `hot_corners` table gets none at all — the wildcard
/// fallback is whole-entry, never a field-by-field merge.
#[test]
fn exact_entry_without_hot_corners_table_gets_none_despite_wildcard() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "*"
        [outputs.hot_corners]
        top_left = "zoom-out"

        [[outputs]]
        name = "HEADLESS-1"
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));

    f.state().check_hot_corners(&output, top_left());

    assert!(
        !fired(&output),
        "an exact entry with no hot_corners table must not fall back to the wildcard's"
    );
}

/// No `[[outputs]]` config at all means no corners ever fire.
#[test]
fn no_output_config_means_no_corners_ever_fire() {
    let mut f = Fixture::new();
    let output = f.add_output(1, (1920, 1080));

    f.state().check_hot_corners(&output, top_left());

    assert!(!fired(&output));
}

/// The activation zone is a square box (both axes must be within `threshold`),
/// not a distance-based radius from the corner point.
#[test]
fn threshold_defines_a_square_zone_not_a_radius() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        threshold = 4
        top_left = "zoom-out"
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));

    f.state()
        .check_hot_corners(&output, Point::from((2.0, 2.0)));
    assert!(fired(&output), "(2, 2) sits inside the 4px top-left zone");

    reset(&output);
    f.state().check_hot_corners(&output, center());
    f.state()
        .check_hot_corners(&output, Point::from((2.0, 5.0)));
    assert!(
        !fired(&output),
        "(2, 5) is past the zone on the y axis: a square box, not a radius"
    );

    f.state().check_hot_corners(&output, center());
    f.state()
        .check_hot_corners(&output, Point::from((5.0, 2.0)));
    assert!(
        !fired(&output),
        "(5, 2) is past the zone on the x axis: a square box, not a radius"
    );

    f.state().check_hot_corners(&output, center());
    f.state()
        .check_hot_corners(&output, Point::from((3.5, 3.5)));
    assert!(
        fired(&output),
        "(3.5, 3.5) is inside the 4px square on both axes but ~4.95px from \
         the corner: a square box, not a radius"
    );
}

/// Each corner maps to its own binding: a config binding only one corner fires
/// for that corner's position and none of the other three.
#[test]
fn every_corner_fires_only_its_own_configured_binding() {
    let corners: [(&str, Point<f64, Logical>); 4] = [
        ("top_left", top_left()),
        ("top_right", top_right()),
        ("bottom_left", bottom_left()),
        ("bottom_right", bottom_right()),
    ];

    for (key, own_pos) in corners {
        let toml = format!(
            "[[outputs]]\nname = \"HEADLESS-1\"\n[outputs.hot_corners]\n{key} = \"zoom-out\"\n"
        );
        let mut f = Fixture::with_config(config(&toml));
        let output = f.add_output(1, (1920, 1080));

        for (other_key, other_pos) in corners {
            if other_key == key {
                continue;
            }
            f.state().check_hot_corners(&output, other_pos);
        }
        assert!(
            !fired(&output),
            "{key}'s binding must not fire for any other corner"
        );

        f.state().check_hot_corners(&output, own_pos);
        assert!(fired(&output), "{key}'s own corner fires its binding");
    }
}

/// Entering a corner fires once; staying inside never refires; leaving (e.g.
/// to the center) and re-entering fires again; moving straight into a
/// different corner fires that corner too.
#[test]
fn corner_entry_latches_until_left_and_reentered() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"
        bottom_right = "zoom-out"
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));

    f.state().check_hot_corners(&output, top_left());
    assert!(fired(&output), "entering the corner fires once");
    reset(&output);

    f.state().check_hot_corners(&output, top_left());
    assert!(
        !fired(&output),
        "staying inside the same corner does not refire"
    );

    f.state().check_hot_corners(&output, center());
    f.state().check_hot_corners(&output, top_left());
    assert!(fired(&output), "leaving and re-entering fires again");
    reset(&output);

    f.state().check_hot_corners(&output, bottom_right());
    assert!(
        fired(&output),
        "moving straight into another corner fires it too"
    );
}

/// `disable_when_fullscreen` (default true) suppresses firing while the
/// output is fullscreen; the suppressed entry still latches, so the pointer
/// must leave and re-enter after fullscreen ends before it fires.
#[test]
fn fullscreen_suppresses_by_default_and_latches_through_exit() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_fullscreen_window(&mut f, id);
    assert!(f.state().stage.has_fullscreen());

    f.state().check_hot_corners(&output, top_left());
    assert!(!fired(&output), "a fullscreen window suppresses by default");

    f.client(id).window(&surface).unset_fullscreen();
    f.double_roundtrip(id);
    f.client(id).window(&surface).ack_last_and_commit();
    f.double_roundtrip(id);
    assert!(!f.state().stage.has_fullscreen());

    f.state().check_hot_corners(&output, top_left());
    assert!(
        !fired(&output),
        "the suppressed entry already latched; fullscreen ending alone must not refire it"
    );

    f.state().check_hot_corners(&output, center());
    f.state().check_hot_corners(&output, top_left());
    assert!(
        fired(&output),
        "leaving and re-entering fires now that fullscreen is gone"
    );
}

/// `disable_when_fullscreen = false` lets a corner fire even on a fullscreen output.
#[test]
fn fullscreen_suppression_can_be_disabled() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"
        disable_when_fullscreen = false
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_fullscreen_window(&mut f, id);
    assert!(f.state().stage.has_fullscreen());

    f.state().check_hot_corners(&output, top_left());

    assert!(
        fired(&output),
        "disabling the guard lets it fire during fullscreen"
    );
}

/// `disable_while_dragging` (default true) suppresses firing while any mouse
/// button is held; the suppressed entry still latches, matching the
/// fullscreen guard's behavior.
#[test]
fn dragging_suppresses_by_default_and_latches_through_release() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));
    f.state().held_buttons.insert(BTN_LEFT);

    f.state().check_hot_corners(&output, top_left());
    assert!(!fired(&output), "a held button suppresses by default");

    f.state().held_buttons.remove(&BTN_LEFT);
    f.state().check_hot_corners(&output, top_left());
    assert!(
        !fired(&output),
        "the suppressed entry already latched; releasing alone must not refire it"
    );

    f.state().check_hot_corners(&output, center());
    f.state().check_hot_corners(&output, top_left());
    assert!(
        fired(&output),
        "leaving and re-entering fires once the button is released"
    );
}

/// `disable_while_dragging = false` lets a corner fire even with a button held.
#[test]
fn dragging_suppression_can_be_disabled() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"
        disable_while_dragging = false
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));
    f.state().held_buttons.insert(BTN_LEFT);

    f.state().check_hot_corners(&output, top_left());

    assert!(
        fired(&output),
        "disabling the guard lets it fire while dragging"
    );
}

/// The entry latch is a single slot shared across outputs: a check on a
/// different output — any position — steals it, so re-entering the first
/// output's corner immediately fires again without an intervening leave.
#[test]
fn cross_output_call_clears_the_single_latch_slot() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"

        [[outputs]]
        name = "HEADLESS-2"
        [outputs.hot_corners]
        top_left = "zoom-out"
        "#,
    ));
    let out1 = f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1920, 1080));

    f.state().check_hot_corners(&out1, top_left());
    assert!(fired(&out1), "out1's corner fires");
    reset(&out1);

    f.state().check_hot_corners(&out1, top_left());
    assert!(!fired(&out1), "staying inside does not refire");

    f.state().check_hot_corners(&out2, center());

    f.state().check_hot_corners(&out1, top_left());
    assert!(
        fired(&out1),
        "the cross-output call cleared out1's latch, so re-entering fires immediately"
    );
}

/// A check on an output with no config entry at all also clears the latch
/// (the "no output config" early-return path, distinct from the cross-output
/// steal above).
#[test]
fn check_on_unbound_output_clears_the_latch() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"
        "#,
    ));
    let out1 = f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1920, 1080));

    f.state().check_hot_corners(&out1, top_left());
    assert!(fired(&out1));
    reset(&out1);

    // HEADLESS-2 has no config entry at all (no exact, no wildcard).
    f.state().check_hot_corners(&out2, top_left());

    f.state().check_hot_corners(&out1, top_left());
    assert!(
        fired(&out1),
        "a check on an unconfigured output clears the latch too"
    );
}

/// A check on an output with its own `[[outputs]]` entry but no `hot_corners`
/// table (empty bindings) also clears the latch — the other early-return path.
#[test]
fn check_on_output_with_empty_bindings_clears_the_latch() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "zoom-out"

        [[outputs]]
        name = "HEADLESS-2"
        "#,
    ));
    let out1 = f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1920, 1080));

    f.state().check_hot_corners(&out1, top_left());
    assert!(fired(&out1));
    reset(&out1);

    // HEADLESS-2 has an entry, but no hot_corners table, so its bindings are empty.
    f.state().check_hot_corners(&out2, top_left());

    f.state().check_hot_corners(&out1, top_left());
    assert!(
        fired(&out1),
        "a check on an output with empty bindings clears the latch too"
    );
}

/// A corner bound to `"none"` never fires, while a sibling corner on the same
/// output does.
#[test]
fn none_leaves_the_corner_unbound_while_a_sibling_still_fires() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "HEADLESS-1"
        [outputs.hot_corners]
        top_left = "none"
        top_right = "zoom-out"
        "#,
    ));
    let output = f.add_output(1, (1920, 1080));

    f.state().check_hot_corners(&output, top_left());
    assert!(!fired(&output), "\"none\" never fires");

    f.state().check_hot_corners(&output, top_right());
    assert!(fired(&output), "a sibling corner still fires");
}
