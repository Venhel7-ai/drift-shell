//! Keyboard-focus delivery timing: focus must never reach a surface before
//! its first buffer, closing a window hands focus down the MRU chain, and a
//! launcher layer hands focus back to the window when it tears down.

use driftwm::config::Config;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::{Fixture, keyboard_focus, server_surface, window_by_app_id};

/// Map one toplevel with a buffer and settle. Returns the client surface.
fn map_window(
    f: &mut Fixture,
    id: super::client::ClientId,
    app_id: &str,
) -> wayland_client::protocol::wl_surface::WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_app_id(app_id);
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.set_size(400, 300);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    surface
}

#[test]
fn no_keyboard_focus_before_first_buffer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.double_roundtrip(id);
    assert_eq!(keyboard_focus(&mut f), None);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let mapped = f
        .state()
        .stage
        .windows()
        .next()
        .and_then(|w| w.client())
        .cloned()
        .unwrap();
    assert_eq!(keyboard_focus(&mut f), Some(server_surface(&mapped)));
}

#[test]
fn focus_after_close_walks_the_mru() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    let c = map_window(&mut f, id, "c");

    let win_a = window_by_app_id(&mut f, "a").unwrap();
    let win_b = window_by_app_id(&mut f, "b").unwrap();
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "c").unwrap()))
    );

    f.client(id).window(&c).destroy();
    f.double_roundtrip(id);
    assert_eq!(keyboard_focus(&mut f), Some(server_surface(&win_b)));

    let b = f.client(id).state.windows[1].surface.clone();
    f.client(id).window(&b).destroy();
    f.double_roundtrip(id);
    assert_eq!(keyboard_focus(&mut f), Some(server_surface(&win_a)));
}

/// A dismissed launcher destroys its layer-shell role while the `wl_surface` is
/// still alive and mapped; the focus recompute must not re-pick it.
#[test]
fn exclusive_layer_teardown_returns_focus_to_the_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, "a");
    let win_a = window_by_app_id(&mut f, "a").unwrap();
    assert_eq!(keyboard_focus(&mut f), Some(server_surface(&win_a)));

    let layer = f
        .client(id)
        .create_layer(None, zwlr_layer_shell_v1::Layer::Overlay, "launcher");
    let ls = layer.surface.clone();
    layer.set_configure_props(super::client::LayerConfigureProps {
        size: Some((400, 300)),
        kb_interactivity: Some(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&ls);
    layer.set_size(400, 300);
    layer.attach_new_buffer();
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_ne!(
        keyboard_focus(&mut f),
        Some(server_surface(&win_a)),
        "an exclusive layer must own focus while it is mapped"
    );

    // Teardown order a real launcher uses: layer-shell role first, wl_surface after.
    f.client(id).layer(&ls).layer_surface.destroy();
    f.double_roundtrip(id);
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&win_a)),
        "focus must return to the window the launcher was opened over"
    );

    f.client(id).layer(&ls).surface.destroy();
    f.double_roundtrip(id);
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&win_a)),
        "focus must stay on the window once the launcher's surface is gone too"
    );
}

#[test]
fn widget_is_skipped_in_the_focus_chain() {
    let mut f = Fixture::with_config(
        Config::from_toml(
            r#"
[[window_rules]]
app_id = "widget"
widget = true
"#,
        )
        .unwrap(),
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, "a");
    map_window(&mut f, id, "widget");
    let b = map_window(&mut f, id, "b");

    let win_a = window_by_app_id(&mut f, "a").unwrap();
    let widget = window_by_app_id(&mut f, "widget").unwrap();

    f.client(id).window(&b).destroy();
    f.double_roundtrip(id);

    // Focus falls through to the other normal window, never the widget.
    assert_eq!(keyboard_focus(&mut f), Some(server_surface(&win_a)));
    assert!(!f.state().stage.focus_history().iter().any(|w| w == &widget));
}
