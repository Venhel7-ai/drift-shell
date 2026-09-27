//! `zwp_virtual_keyboard_v1` (`src/protocols/virtual_keyboard.rs`): a virtual
//! key resolves against the virtual keyboard's *own* keymap and modifiers, a
//! bound combo runs the compositor action with press and release swallowed,
//! and anything else reaches the focused client under the virtual keyboard's
//! keymap, which a physical key afterwards must find restored to the seat's.
//! A seat layout reload overwrites that keymap, so the next virtual key sends
//! it again.

use smithay::input::keyboard::xkb;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::{
    self, ZwpVirtualKeyboardV1,
};

use super::client::{ClientId, KeyboardEvent};
use super::input_backend::{key_press, key_release};
use super::{Fixture, config, keyboard_focus, map_window, server_surface, window_by_app_id};

const KEY_A: u32 = 30;
const KEY_EQUAL: u32 = 13;
const KEY_B: u32 = 48;
const KEY_Y: u32 = 21;

fn compile_keymap(layout: &str) -> xkb::Keymap {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .expect("compile a keymap")
}

/// `b`'s virtual keyboard presses and releases `KEY_A` into `a`'s focused
/// window under a `de` keymap — not the seat's `us`, so a client holding it
/// is observable. Returns the keyboard and the keymap text.
fn press_a_through_virtual_keyboard(
    f: &mut Fixture,
    a: ClientId,
    b: ClientId,
) -> (ZwpVirtualKeyboardV1, String) {
    map_window(f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(f),
        Some(server_surface(&window_by_app_id(f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let vk = f.client(b).create_virtual_keyboard();
    let text = compile_keymap("de").get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    f.client(b).virtual_keyboard_keymap(&vk, &text);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_A, true);
    f.client(b).virtual_keyboard_key(&vk, 2, KEY_A, false);
    f.roundtrip(b);
    f.roundtrip(a);

    (vk, text)
}

#[test]
fn a_virtual_key_reaches_the_focused_window_with_the_virtual_keyboards_keymap_first() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    let b = f.add_client();

    let (_vk, text) = press_a_through_virtual_keyboard(&mut f, a, b);

    assert_eq!(
        f.client(a).drain_keyboard_events(),
        vec![
            // A keymap change carries a modifiers resync — here the virtual
            // keyboard's default, nothing depressed.
            KeyboardEvent::Keymap(text),
            KeyboardEvent::Modifiers { mods_depressed: 0 },
            KeyboardEvent::Key {
                key: KEY_A,
                state: 1
            },
            KeyboardEvent::Key {
                key: KEY_A,
                state: 0
            },
        ],
        "the client must see the virtual keyboard's keymap exactly once, then \
         the press and the release — a second key must not re-send a keymap \
         the client already holds"
    );
}

#[test]
fn an_on_screen_keyboard_uploading_the_seats_own_layout_costs_the_client_nothing() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    // The keyboard binds off the seat global, which only shows up after the
    // first roundtrip; a second one sees the keymap that bind provokes.
    f.double_roundtrip(a);

    let initial = f.client(a).drain_keyboard_events();
    let Some(KeyboardEvent::Keymap(seat_text)) = initial.into_iter().next() else {
        panic!("expected the seat's keymap on the client's first contact");
    };
    let us_text = compile_keymap("us").get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    assert_eq!(
        seat_text, us_text,
        "precondition: the fixture's seat layout must be `us`, or uploading a \
         `us` keymap below would not actually match what the client already holds"
    );

    let b = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let vk = f.client(b).create_virtual_keyboard();
    f.client(b).virtual_keyboard_keymap(&vk, &us_text);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_A, true);
    f.client(b).virtual_keyboard_key(&vk, 2, KEY_A, false);
    f.roundtrip(b);
    f.roundtrip(a);

    assert_eq!(
        f.client(a).drain_keyboard_events(),
        vec![
            KeyboardEvent::Key {
                key: KEY_A,
                state: 1
            },
            KeyboardEvent::Key {
                key: KEY_A,
                state: 0
            },
        ],
        "uploading the seat's own layout must cost the client nothing but the \
         key itself — no keymap re-send, and therefore no resync modifiers"
    );

    key_press(&mut f, KEY_B);
    key_release(&mut f, KEY_B);
    f.roundtrip(a);

    assert_eq!(
        f.client(a).drain_keyboard_events(),
        vec![
            KeyboardEvent::Key {
                key: KEY_B,
                state: 1
            },
            KeyboardEvent::Key {
                key: KEY_B,
                state: 0
            },
        ],
        "a physical key afterwards must find nothing to restore, since the \
         client's keymap never diverged from the seat's"
    );
}

#[test]
fn a_virtual_key_matching_a_compositor_binding_runs_the_action_and_never_reaches_the_client() {
    let mut f = Fixture::with_config(config(
        r#"
        [keybindings]
        "ctrl+alt+equal" = "zoom-out"
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let resting_zoom = f.state().zoom_target();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    let keymap = compile_keymap("us");
    let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    f.client(b).virtual_keyboard_keymap(&vk, &text);
    f.roundtrip(b);

    let ctrl = keymap.mod_get_index(xkb::MOD_NAME_CTRL);
    let alt = keymap.mod_get_index(xkb::MOD_NAME_ALT);
    let mask = (1u32 << ctrl) | (1u32 << alt);
    f.client(b).virtual_keyboard_modifiers(&vk, mask);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_EQUAL, true);
    f.client(b).virtual_keyboard_key(&vk, 2, KEY_EQUAL, false);
    f.roundtrip(b);
    f.roundtrip(a);

    assert_ne!(
        f.state().zoom_target(),
        resting_zoom,
        "a virtual key matching a compositor binding must run its action"
    );
    assert!(
        !f.client(a)
            .keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Key { .. })),
        "a key a compositor binding consumes must never reach the focused \
         client, neither the press nor its paired release: {:?}",
        f.client(a).keyboard_events()
    );
}

#[test]
fn a_bound_combo_still_fires_after_a_keymap_reupload_that_follows_modifiers() {
    let mut f = Fixture::with_config(config(
        r#"
        [keybindings]
        "ctrl+alt+equal" = "zoom-out"
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let resting_zoom = f.state().zoom_target();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    let k1 = compile_keymap("us");
    f.client(b)
        .virtual_keyboard_keymap(&vk, &k1.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1));
    f.roundtrip(b);

    let ctrl = k1.mod_get_index(xkb::MOD_NAME_CTRL);
    let alt = k1.mod_get_index(xkb::MOD_NAME_ALT);
    let mask = (1u32 << ctrl) | (1u32 << alt);
    f.client(b).virtual_keyboard_modifiers(&vk, mask);
    f.roundtrip(b);

    // `gb`, not `de`: both differ from `us` as text, but `de` makes the
    // physical `=` key a dead accent at the base level, which would fail the
    // binding on the sym alone — this scenario must fail only if the
    // *modifiers* are lost across the re-upload.
    let k2 = compile_keymap("gb");
    f.client(b)
        .virtual_keyboard_keymap(&vk, &k2.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1));
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_EQUAL, true);
    f.client(b).virtual_keyboard_key(&vk, 2, KEY_EQUAL, false);
    f.roundtrip(b);
    f.roundtrip(a);

    assert_ne!(
        f.state().zoom_target(),
        resting_zoom,
        "a bound combo must still fire after a keymap re-upload that follows \
         the modifiers request"
    );
    assert!(
        !f.client(a)
            .keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Key { .. })),
        "a key a compositor binding consumes must never reach the focused \
         client, even after the keymap that resolved it was replaced: {:?}",
        f.client(a).keyboard_events()
    );
}

#[test]
fn a_release_is_swallowed_across_a_keymap_reupload() {
    let mut f = Fixture::with_config(config(
        r#"
        [keybindings]
        "ctrl+alt+equal" = "zoom-out"
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let resting_zoom = f.state().zoom_target();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    let k1 = compile_keymap("us");
    f.client(b)
        .virtual_keyboard_keymap(&vk, &k1.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1));
    f.roundtrip(b);

    let ctrl = k1.mod_get_index(xkb::MOD_NAME_CTRL);
    let alt = k1.mod_get_index(xkb::MOD_NAME_ALT);
    let mask = (1u32 << ctrl) | (1u32 << alt);
    f.client(b).virtual_keyboard_modifiers(&vk, mask);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_EQUAL, true);
    f.roundtrip(b);
    f.roundtrip(a);
    assert_ne!(
        f.state().zoom_target(),
        resting_zoom,
        "precondition: the press must run the bound action, so its release \
         has something to swallow"
    );

    let k2 = compile_keymap("de");
    f.client(b)
        .virtual_keyboard_keymap(&vk, &k2.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1));
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 2, KEY_EQUAL, false);
    f.roundtrip(b);
    f.roundtrip(a);

    assert!(
        !f.client(a)
            .keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Key { .. })),
        "the release paired with a swallowed press must stay swallowed across \
         a keymap re-upload — neither it nor the press it belongs to may \
         reach the client: {:?}",
        f.client(a).keyboard_events()
    );
}

#[test]
fn a_physical_key_after_virtual_typing_restores_the_seat_keymap_first() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    let b = f.add_client();

    let (_vk, virtual_text) = press_a_through_virtual_keyboard(&mut f, a, b);
    // Leave only what the physical key produces.
    f.client(a).drain_keyboard_events();

    key_press(&mut f, KEY_B);
    key_release(&mut f, KEY_B);
    f.roundtrip(a);

    let events = f.client(a).drain_keyboard_events();
    let Some(KeyboardEvent::Keymap(seat_text)) = events.first() else {
        panic!("expected the seat's keymap to go out before the physical key, got {events:?}");
    };
    assert_ne!(
        seat_text, &virtual_text,
        "a physical key must find the seat's own keymap restored, not the one \
         the virtual keyboard left the client holding"
    );

    let key_events: Vec<&KeyboardEvent> = events
        .iter()
        .filter(|e| matches!(e, KeyboardEvent::Key { .. }))
        .collect();
    assert_eq!(
        key_events,
        vec![
            &KeyboardEvent::Key {
                key: KEY_B,
                state: 1
            },
            &KeyboardEvent::Key {
                key: KEY_B,
                state: 0
            },
        ],
        "the physical press and release must still reach the client, after \
         the keymap reset"
    );
}

#[test]
fn key_before_keymap_is_a_protocol_error() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let b = f.add_client();
    f.roundtrip(b);
    let vk = f.client(b).create_virtual_keyboard();

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_A, true);
    f.pump(10);

    let error = f
        .client(b)
        .protocol_error()
        .expect("`key` before `keymap` must be a protocol error");
    assert_eq!(error.object_interface, "zwp_virtual_keyboard_v1");
    assert_eq!(
        error.code,
        zwp_virtual_keyboard_v1::Error::NoKeymap as u32,
        "wrong error code, got: {}",
        error.message
    );

    f.kill_client(b);
}

#[test]
fn a_modifiers_request_reaches_the_focused_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    let keymap = compile_keymap("de");
    f.client(b)
        .virtual_keyboard_keymap(&vk, &keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1));
    f.roundtrip(b);

    let ctrl = keymap.mod_get_index(xkb::MOD_NAME_CTRL);
    let mask = 1u32 << ctrl;
    f.client(b).virtual_keyboard_modifiers(&vk, mask);
    f.roundtrip(b);
    f.roundtrip(a);

    assert!(
        f.client(a)
            .drain_keyboard_events()
            .contains(&KeyboardEvent::Modifiers {
                mods_depressed: mask
            }),
        "a `modifiers` request must reach the focused window's wl_keyboard, \
         possibly preceded by the keymap on first contact"
    );
}

#[test]
fn destroying_a_virtual_keyboard_frees_its_bookkeeping() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    let b = f.add_client();

    let (vk, _text) = press_a_through_virtual_keyboard(&mut f, a, b);
    assert_eq!(
        f.state().virtual_kb_bindings.keyboard_count(),
        1,
        "precondition: the manager registered the virtual keyboard"
    );

    f.client(b).virtual_keyboard_destroy(&vk);
    f.roundtrip(b);

    assert_eq!(
        f.state().virtual_kb_bindings.keyboard_count(),
        0,
        "destroying the virtual keyboard must free its compositor-side bookkeeping"
    );
}

#[test]
fn a_layout_reload_resends_a_held_virtual_keymap_before_the_next_virtual_key() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    let b = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let vk = f.client(b).create_virtual_keyboard();
    let text = compile_keymap("gb").get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    f.client(b).virtual_keyboard_keymap(&vk, &text);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_A, true);
    f.client(b).virtual_keyboard_key(&vk, 2, KEY_A, false);
    f.roundtrip(b);
    f.roundtrip(a);
    assert!(
        matches!(
            f.client(a).drain_keyboard_events().first(),
            Some(KeyboardEvent::Keymap(held)) if *held == text
        ),
        "precondition: the first virtual key leaves the client holding the \
         virtual keyboard's keymap"
    );

    f.state()
        .reload_config_from_contents("[input.keyboard]\nlayout = \"de\"\n");
    // The headless fixture has no backend to drain a queued mode intent.
    f.state().pending_mode_changes.clear();
    f.roundtrip(a);
    assert!(
        f.client(a)
            .drain_keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Keymap(_))),
        "precondition: the reloaded seat layout is broadcast to every \
         wl_keyboard, overwriting the virtual keymap the client held"
    );

    f.client(b).virtual_keyboard_key(&vk, 3, KEY_A, true);
    f.client(b).virtual_keyboard_key(&vk, 4, KEY_A, false);
    f.roundtrip(b);
    f.roundtrip(a);

    assert_eq!(
        f.client(a).drain_keyboard_events(),
        vec![
            KeyboardEvent::Keymap(text),
            KeyboardEvent::Modifiers { mods_depressed: 0 },
            KeyboardEvent::Key {
                key: KEY_A,
                state: 1
            },
            KeyboardEvent::Key {
                key: KEY_A,
                state: 0
            },
        ],
        "a layout reload takes the virtual keymap off the client, so the next \
         virtual key must hand it back before the key it decodes"
    );
}

#[test]
fn a_layout_group_selected_before_a_keymap_reupload_still_resolves_the_bound_sym() {
    let mut f = Fixture::with_config(config(
        r#"
        [keybindings]
        "ctrl+z" = "zoom-out"
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window_by_app_id(&mut f, "typist").unwrap())),
        "precondition: the mapped window holds keyboard focus"
    );
    f.client(a).drain_keyboard_events();

    let resting_zoom = f.state().zoom_target();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    // Two groups, and only the second one — `de` — puts the bound `z` on the
    // physical `y` key, so the binding fires from the group alone.
    let keymap = compile_keymap("us,de");
    let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    f.client(b).virtual_keyboard_keymap(&vk, &text);
    f.roundtrip(b);

    let ctrl = keymap.mod_get_index(xkb::MOD_NAME_CTRL);
    f.client(b)
        .virtual_keyboard_modifiers_in_group(&vk, 1u32 << ctrl, 1);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_keymap(&vk, &text);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_Y, true);
    f.client(b).virtual_keyboard_key(&vk, 2, KEY_Y, false);
    f.roundtrip(b);
    f.roundtrip(a);

    assert_ne!(
        f.state().zoom_target(),
        resting_zoom,
        "a re-upload replaces the xkb state, and the layout group the \
         modifiers request selected must go back onto it — otherwise the key \
         resolves under the first group and the bound zoom never runs"
    );
    assert!(
        !f.client(a)
            .keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Key { .. })),
        "a key a compositor binding consumes must never reach the focused \
         client: {:?}",
        f.client(a).keyboard_events()
    );
}

/// The seat's-own-layout path deliberately sends no keymap, so it records no
/// keymap to restore either — but the modifiers an on-screen keyboard sends
/// still reach the client, and a held on-screen Shift must not outlive the
/// next physical key.
#[test]
fn a_physical_key_resets_the_modifiers_an_identical_keymap_upload_left_held() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    f.double_roundtrip(a);
    let Some(KeyboardEvent::Keymap(seat_text)) =
        f.client(a).drain_keyboard_events().into_iter().next()
    else {
        panic!("expected the seat's keymap on the client's first contact");
    };
    let us = compile_keymap("us");
    assert_eq!(
        seat_text,
        us.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1),
        "precondition: the fixture's seat layout must be `us`"
    );

    let b = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    f.client(a).drain_keyboard_events();

    let vk = f.client(b).create_virtual_keyboard();
    f.client(b).virtual_keyboard_keymap(&vk, &seat_text);
    f.roundtrip(b);

    let shift = 1u32 << us.mod_get_index(xkb::MOD_NAME_SHIFT);
    f.client(b).virtual_keyboard_modifiers(&vk, shift);
    f.roundtrip(b);
    f.roundtrip(a);
    assert_eq!(
        f.client(a).drain_keyboard_events(),
        vec![KeyboardEvent::Modifiers {
            mods_depressed: shift
        }],
        "precondition: the on-screen Shift reaches the client, with no keymap \
         re-send since the layout is the seat's own"
    );

    key_press(&mut f, KEY_B);
    key_release(&mut f, KEY_B);
    f.roundtrip(a);

    let events = f.client(a).drain_keyboard_events();
    let reset = events
        .iter()
        .position(|e| matches!(e, KeyboardEvent::Modifiers { mods_depressed: 0 }));
    let key = events.iter().position(|e| {
        matches!(
            e,
            KeyboardEvent::Key {
                key: KEY_B,
                state: 1
            }
        )
    });
    assert!(
        matches!((reset, key), (Some(r), Some(k)) if r < k),
        "the physical key must find the seat's modifiers put back before it, \
         or the client types it under the on-screen Shift: {events:?}"
    );
}

/// A virtual keyboard destroyed with a forwarded key still down must release
/// it — `wl_keyboard` has no event for a vanished source, so the client would
/// otherwise hold the key, and repeat it, until it lost focus.
#[test]
fn destroying_a_virtual_keyboard_releases_the_key_it_left_pressed() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    f.client(a).drain_keyboard_events();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    let text = compile_keymap("de").get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    f.client(b).virtual_keyboard_keymap(&vk, &text);
    f.roundtrip(b);

    f.client(b).virtual_keyboard_key(&vk, 1, KEY_A, true);
    f.roundtrip(b);
    f.roundtrip(a);
    assert!(
        f.client(a)
            .drain_keyboard_events()
            .contains(&KeyboardEvent::Key {
                key: KEY_A,
                state: 1
            }),
        "precondition: the press reached the client"
    );

    f.client(b).virtual_keyboard_destroy(&vk);
    f.roundtrip(b);
    f.roundtrip(a);

    assert!(
        f.client(a)
            .drain_keyboard_events()
            .contains(&KeyboardEvent::Key {
                key: KEY_A,
                state: 0
            }),
        "the teardown must release the key the virtual keyboard left pressed"
    );
}

/// A bound press never reached the client, so its teardown owes no release —
/// one would be a release without a press.
#[test]
fn destroying_a_virtual_keyboard_does_not_release_a_swallowed_key() {
    let mut f = Fixture::with_config(config(
        r#"
        [keybindings]
        "ctrl+alt+equal" = "zoom-out"
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let a = f.add_client();
    map_window(&mut f, a, "typist", (400, 300));
    f.client(a).drain_keyboard_events();

    let b = f.add_client();
    let vk = f.client(b).create_virtual_keyboard();
    let keymap = compile_keymap("us");
    f.client(b)
        .virtual_keyboard_keymap(&vk, &keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1));
    f.roundtrip(b);

    let ctrl = 1u32 << keymap.mod_get_index(xkb::MOD_NAME_CTRL);
    let alt = 1u32 << keymap.mod_get_index(xkb::MOD_NAME_ALT);
    f.client(b).virtual_keyboard_modifiers(&vk, ctrl | alt);
    f.client(b).virtual_keyboard_key(&vk, 1, KEY_EQUAL, true);
    f.roundtrip(b);
    f.roundtrip(a);
    assert!(
        !f.client(a)
            .drain_keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Key { key: KEY_EQUAL, .. })),
        "precondition: the bound press was swallowed"
    );

    f.client(b).virtual_keyboard_destroy(&vk);
    f.roundtrip(b);
    f.roundtrip(a);

    assert!(
        !f.client(a)
            .drain_keyboard_events()
            .iter()
            .any(|e| matches!(e, KeyboardEvent::Key { key: KEY_EQUAL, .. })),
        "a swallowed press owes no release; the client never saw the press"
    );
}
