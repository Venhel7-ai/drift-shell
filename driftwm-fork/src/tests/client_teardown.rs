//! Abrupt client death in every window state. The compositor must reap all
//! traces without panicking — every server dispatch in the fixture already
//! runs the stage invariant check, so a stale entry aborts the test even
//! without an explicit assert.

use driftwm::config::Config;

use super::Fixture;

/// Map one toplevel with a buffer at `size` and settle.
fn map_window(f: &mut Fixture, id: super::client::ClientId, size: (u16, u16)) {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.set_size(size.0, size.1);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);
}

#[test]
fn kill_with_mapped_window_reaps_it() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, (400, 300));
    assert_eq!(f.state().stage.windows().count(), 1);

    f.kill_client(id);
    f.pump(10);
    assert_eq!(f.state().stage.windows().count(), 0);
}

#[test]
fn kill_while_fullscreen_reaps_entry_and_viewport_return() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, (800, 600));
    let surface = f.client(id).state.windows[0].surface.clone();
    let window = f.client(id).window(&surface);
    window.set_fullscreen(None);
    f.double_roundtrip(id);
    f.client(id).window(&surface).ack_last_and_commit();
    f.double_roundtrip(id);
    assert!(f.state().stage.has_fullscreen());

    f.kill_client(id);
    f.pump(10);
    assert!(!f.state().stage.has_fullscreen());
    assert_eq!(f.state().stage.windows().count(), 0);
    let outputs: Vec<_> = f.state().space.outputs().cloned().collect();
    for output in outputs {
        assert!(
            crate::state::output_state(&output)
                .fullscreen_return
                .is_none()
        );
    }
}

#[test]
fn kill_while_pinned_reaps_pin() {
    let mut f = Fixture::with_config(
        Config::from_toml(
            r#"
[[window_rules]]
app_id = "pin"
pinned_to_screen = true
"#,
        )
        .unwrap(),
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_app_id("pin");
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.set_size(300, 200);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    assert!(f.state().stage.has_pinned());

    f.kill_client(id);
    f.pump(10);
    assert!(!f.state().stage.has_pinned());
    assert_eq!(f.state().stage.windows().count(), 0);
}

#[test]
fn kill_mid_configure_does_not_panic() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, (800, 600));
    let surface = f.client(id).state.windows[0].surface.clone();
    // Send a fullscreen configure the client will never ack.
    f.client(id).window(&surface).set_fullscreen(None);
    f.double_roundtrip(id);

    f.kill_client(id);
    f.pump(10);
    assert_eq!(f.state().stage.windows().count(), 0);
    assert!(!f.state().stage.has_fullscreen());
}
