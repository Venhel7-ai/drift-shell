//! Virtual keyboard (`zwp_virtual_keyboard_v1`) with compositor keybindings.
//!
//! smithay's implementation delivers virtual key events straight to the
//! focused client, so an on-screen keyboard could never trigger compositor
//! bindings, and its dispatch is a blanket impl a compositor cannot wrap. This
//! module is smithay's `wayland::virtual_keyboard` vendored (rev `4cf0b620`,
//! MIT; notice at the end of the file) with one addition: each key press
//! first runs through [`VirtualKeyboardBindingHandler::virtual_key_binding`]
//! — resolved against the virtual keyboard's *own* uploaded keymap and
//! modifier state, which need not match the physical layout — and a bound
//! combo executes instead of reaching the focused client (the paired release
//! is swallowed too).
//!
//! smithay tracks which keymap each client holds in a crate-private field and
//! re-sends the seat's before the next physical key, so the copy keeps its own
//! record of the `wl_keyboard`s holding a virtual keymap and the input path
//! calls [`restore_seat_keymap`] before forwarding a physical key.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::rc::Rc;

use smithay::input::keyboard::{
    KeyboardHandle, KeyboardTarget, KeymapFile, Keysym, ModifiersState, xkb,
};
use smithay::input::{Seat, SeatHandler};
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
    zwp_virtual_keyboard_manager_v1::{self, ZwpVirtualKeyboardManagerV1},
    zwp_virtual_keyboard_v1::{self, ZwpVirtualKeyboardV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
    backend::{ClientId, GlobalId, ObjectId},
    protocol::wl_keyboard::{self, KeymapFormat, WlKeyboard},
};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

const MANAGER_VERSION: u32 = 1;

pub trait VirtualKeyboardBindingHandler {
    fn virtual_keyboard_bindings(&mut self) -> &mut VirtualKeyboardBindings;

    /// Execute the compositor binding for `modifiers` + `sym`, if any.
    /// Returns `true` when a binding consumed the key press (it must not
    /// reach the focused client).
    fn virtual_key_binding(&mut self, modifiers: &ModifiersState, sym: Keysym) -> bool;
}

#[derive(Debug)]
pub struct VirtualKeyboardManagerState {
    global: GlobalId,
}

pub struct VirtualKeyboardManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

#[derive(Debug)]
pub struct VirtualKeyboardManagerUserData;

/// Holds only the seat: the keymap and modifier state live in
/// [`VirtualKeyboardBindings`], keyed by the resource, so the xkb state never
/// has to cross threads.
#[derive(Debug)]
pub struct VirtualKeyboardUserData<D: SeatHandler> {
    seat: Seat<D>,
}

impl VirtualKeyboardManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwpVirtualKeyboardManagerV1, VirtualKeyboardManagerGlobalData>,
        D: Dispatch<ZwpVirtualKeyboardManagerV1, VirtualKeyboardManagerUserData>,
        D: Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<D>>,
        D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let data = VirtualKeyboardManagerGlobalData {
            filter: Box::new(filter),
        };
        let global =
            display.create_global::<D, ZwpVirtualKeyboardManagerV1, _>(MANAGER_VERSION, data);
        Self { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// Per-virtual-keyboard xkb state mirrored from the client's `keymap` and
/// `modifiers` requests, keyed by resource so multiple virtual keyboards don't
/// mix layouts, plus which `wl_keyboard`s currently hold a keymap other than
/// the seat's.
///
/// Keymaps are compared by text so an on-screen keyboard that uploads the
/// seat's own layout costs the client nothing: no keymap event, no recompile,
/// no modifiers event resetting what the physical keyboard holds.
#[derive(Default)]
pub struct VirtualKeyboardBindings {
    keyboards: HashMap<ObjectId, VirtualKeyboard>,
    foreign_keymaps: Vec<(Weak<WlKeyboard>, Rc<str>)>,
    /// The focused client holds a modifier mask a virtual keyboard set. The
    /// keymap records cannot stand in for this: a virtual keymap that reads the
    /// same as the seat's records nothing, while its modifiers reach the client
    /// all the same.
    virtual_mods_sent: bool,
    seat_keymap: Option<SeatKeymap>,
}

struct SeatKeymap {
    text: Rc<str>,
    file: KeymapFile,
}

#[derive(Default)]
struct VirtualKeyboard {
    keymap: Option<VirtualKeymap>,
    /// Keycodes whose press a binding consumed; their release must be
    /// swallowed too, or the client sees a release without a press.
    swallowed: HashSet<u32>,
    /// Keycodes forwarded to the focused client as pressed and not yet
    /// released, for [`release_forwarded`] to lift on destroy.
    forwarded: HashSet<u32>,
}

struct VirtualKeymap {
    file: KeymapFile,
    text: Rc<str>,
    state: xkb::State,
    mods: ModifiersState,
}

/// Far above any real xkb keymap (~100 KB), far below an allocation a hostile
/// `size` could weaponize — the wire value goes straight into a buffer.
const MAX_KEYMAP_SIZE: usize = 8 * 1024 * 1024;

impl VirtualKeyboardBindings {
    /// Number of live virtual keyboards (for leak diagnostics).
    pub fn keyboard_count(&self) -> usize {
        self.keyboards.len()
    }

    /// Number of live `wl_keyboard`s holding a virtual keymap (for leak
    /// diagnostics).
    pub fn held_keymap_count(&self) -> usize {
        self.foreign_keymaps
            .iter()
            .filter(|(kbd, _)| kbd.upgrade().is_ok())
            .count()
    }

    fn track_keymap(&mut self, id: ObjectId, format: u32, fd: &OwnedFd, size: usize) {
        if format != KeymapFormat::XkbV1 as u32 {
            tracing::debug!("virtual keyboard: unsupported keymap format {format}");
            return;
        }
        if size > MAX_KEYMAP_SIZE {
            tracing::warn!("virtual keyboard: keymap size {size} exceeds limit, ignoring");
            return;
        }
        let Ok(fd) = fd.try_clone() else {
            return;
        };
        let file = File::from(fd);
        let mut buf = vec![0u8; size];
        if file.read_exact_at(&mut buf, 0).is_err() {
            tracing::warn!("virtual keyboard: failed to read keymap fd");
            return;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let Ok(string) = std::str::from_utf8(&buf[..len]) else {
            tracing::warn!("virtual keyboard: keymap is not valid UTF-8");
            return;
        };
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let Some(keymap) = xkb::Keymap::new_from_string(
            &context,
            string.to_string(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) else {
            tracing::warn!("virtual keyboard: failed to compile keymap");
            return;
        };
        let Some(kb) = self.keyboards.get_mut(&id) else {
            return;
        };
        // A keymap re-upload (e.g. a layout switch) replaces the xkb state but
        // keeps the modifiers and the swallowed set: a key pressed under the old
        // keymap still owes its release a swallow. The modifier mask goes onto
        // the new state as well — the client applies the mask it is sent to the
        // keymap it is sent, and the sym a binding is resolved from must agree.
        let mut mods = kb.keymap.take().map(|k| k.mods).unwrap_or_default();
        let mut state = xkb::State::new(&keymap);
        let mask = mods.serialized;
        state.update_mask(
            mask.depressed,
            mask.latched,
            mask.locked,
            0,
            0,
            mask.layout_effective,
        );
        mods.update_with(&state);
        // The canonical serialization, not the upload's bytes: `seat_keymap`
        // records `get_as_string` too, and the two are compared verbatim to
        // decide whether a restore is owed.
        let text: Rc<str> = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1).into();
        kb.keymap = Some(VirtualKeymap {
            file: KeymapFile::new(&keymap),
            text,
            state,
            mods,
        });
    }

    fn track_modifiers(
        &mut self,
        id: ObjectId,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) {
        if let Some(keymap) = self
            .keyboards
            .get_mut(&id)
            .and_then(|kb| kb.keymap.as_mut())
        {
            keymap
                .state
                .update_mask(depressed, latched, locked, 0, 0, group);
            keymap.mods.update_with(&keymap.state);
        }
    }

    fn has_keymap(&self, id: &ObjectId) -> bool {
        self.keyboards.get(id).is_some_and(|kb| kb.keymap.is_some())
    }

    fn mods(&self, id: &ObjectId) -> Option<ModifiersState> {
        self.keyboards
            .get(id)
            .and_then(|kb| kb.keymap.as_ref())
            .map(|keymap| keymap.mods)
    }
}

/// Resolve a virtual `key` event against the keyboard's mirrored xkb state and
/// hand it to the handler's binding lookup. Returns `true` when the event was
/// consumed (a bound press, or the release paired with one).
fn handle_key<D: VirtualKeyboardBindingHandler>(
    state: &mut D,
    id: ObjectId,
    key: u32,
    key_state: u32,
) -> bool {
    let Some(kb) = state.virtual_keyboard_bindings().keyboards.get_mut(&id) else {
        return false;
    };
    let pressed = key_state == 1;
    if !pressed {
        return kb.swallowed.remove(&key);
    }
    let Some(keymap) = kb.keymap.as_ref() else {
        return false;
    };
    // Raw evdev keycode (wl_keyboard coding) → xkb keycode space.
    let sym = keymap.state.key_get_one_sym(xkb::Keycode::new(key + 8));
    // The modifiers the client would be told, so a combo the compositor
    // consumes and one that reaches the client never disagree.
    let modifiers = keymap.mods;
    if !state.virtual_key_binding(&modifiers, sym) {
        return false;
    }
    if let Some(kb) = state.virtual_keyboard_bindings().keyboards.get_mut(&id) {
        kb.swallowed.insert(key);
    }
    true
}

fn focused_client<D>(keyboard: &KeyboardHandle<D>) -> Option<(D::KeyboardFocus, Client)>
where
    D: SeatHandler + 'static,
    D::KeyboardFocus: WaylandFocus + Clone,
{
    let focus = keyboard.current_focus()?;
    let client = focus.wl_surface()?.client()?;
    Some((focus, client))
}

/// Mark the focused client as carrying a virtual modifier mask, so
/// [`restore_seat_keymap`] puts the seat's back even with no keymap to restore
/// alongside. A mask equal to the seat's own is an input method echoing what
/// the seat told it; the client already matches, so it is skipped.
fn record_virtual_mods<D>(state: &mut D, keyboard: &KeyboardHandle<D>, mods: ModifiersState)
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
{
    if mods.serialized != keyboard.modifier_state().serialized {
        state.virtual_keyboard_bindings().virtual_mods_sent = true;
    }
}

fn seat_keymap<D>(state: &mut D, keyboard: &KeyboardHandle<D>) -> Rc<str>
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
{
    if let Some(seat) = &state.virtual_keyboard_bindings().seat_keymap {
        return seat.text.clone();
    }
    let (text, file) = keyboard.with_xkb_state(state, |context| {
        let xkb = context.xkb().lock().unwrap();
        // SAFETY: both calls copy the keymap out — the text into its own
        // string, `KeymapFile::new` into a sealed file — and keep no reference.
        let keymap = unsafe { xkb.keymap() };
        (
            keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1),
            KeymapFile::new(keymap),
        )
    });
    let text: Rc<str> = text.into();
    state.virtual_keyboard_bindings().seat_keymap = Some(SeatKeymap {
        text: text.clone(),
        file,
    });
    text
}

/// Send the virtual keyboard's keymap to the focused client's `wl_keyboard`s
/// that don't hold it yet, then its modifiers, which a keymap change must
/// carry. Returns whether any keymap went out.
fn send_keymap<D>(
    state: &mut D,
    seat: &Seat<D>,
    keyboard: &KeyboardHandle<D>,
    id: ObjectId,
    focus: &D::KeyboardFocus,
    client: &Client,
) -> bool
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
    D::KeyboardFocus: WaylandFocus,
{
    let kbds: Vec<WlKeyboard> = keyboard.client_keyboards(client).collect();
    let seat_text = seat_keymap(state, keyboard);
    let bindings = state.virtual_keyboard_bindings();
    let Some(keymap) = bindings
        .keyboards
        .get(&id)
        .and_then(|kb| kb.keymap.as_ref())
    else {
        return false;
    };
    let mut sent = false;
    for kbd in kbds {
        let held = bindings
            .foreign_keymaps
            .iter()
            .find(|(held, _)| held.upgrade().is_ok_and(|h| h == kbd))
            .map_or(&seat_text, |(_, text)| text);
        if **held == *keymap.text {
            continue;
        }
        if let Err(err) = keymap.file.send(&kbd) {
            tracing::warn!("virtual keyboard: failed to send keymap to client: {err}");
            continue;
        }
        bindings
            .foreign_keymaps
            .retain(|(held, _)| held.upgrade().is_ok_and(|h| h != kbd));
        // A keymap that reads the same as the seat's leaves nothing to restore.
        if *keymap.text != *seat_text {
            bindings
                .foreign_keymaps
                .push((kbd.downgrade(), keymap.text.clone()));
        }
        sent = true;
    }
    let mods = keymap.mods;
    if sent {
        focus.modifiers(seat, state, mods, SERIAL_COUNTER.next_serial());
        record_virtual_mods(state, keyboard, mods);
    }
    sent
}

fn deliver_key<D>(state: &mut D, seat: &Seat<D>, id: ObjectId, time: u32, key: u32, key_state: u32)
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
    D::KeyboardFocus: WaylandFocus + Clone,
{
    let Some(keyboard) = seat.get_keyboard() else {
        return;
    };
    let Some((focus, client)) = focused_client(&keyboard) else {
        return;
    };
    send_keymap(state, seat, &keyboard, id.clone(), &focus, &client);
    // The protocol does not declare the argument as an enum.
    let key_state = if key_state == 1 {
        wl_keyboard::KeyState::Pressed
    } else {
        wl_keyboard::KeyState::Released
    };
    for kbd in keyboard.client_keyboards(&client) {
        kbd.key(SERIAL_COUNTER.next_serial().into(), time, key, key_state);
    }
    if let Some(kb) = state.virtual_keyboard_bindings().keyboards.get_mut(&id) {
        if key_state == wl_keyboard::KeyState::Pressed {
            kb.forwarded.insert(key);
        } else {
            kb.forwarded.remove(&key);
        }
    }
}

/// Release whatever a departing virtual keyboard left pressed in the focused
/// client: `wl_keyboard` has no event for a vanished source, so the key would
/// stay down — and repeating, client-side — until the window lost focus.
fn release_forwarded<D>(seat: &Seat<D>, forwarded: &HashSet<u32>)
where
    D: SeatHandler + 'static,
    D::KeyboardFocus: WaylandFocus + Clone,
{
    if forwarded.is_empty() {
        return;
    }
    let Some(keyboard) = seat.get_keyboard() else {
        return;
    };
    let Some((_, client)) = focused_client(&keyboard) else {
        return;
    };
    let time = smithay::utils::Clock::<smithay::utils::Monotonic>::new()
        .now()
        .as_millis();
    for kbd in keyboard.client_keyboards(&client) {
        for key in forwarded {
            kbd.key(
                SERIAL_COUNTER.next_serial().into(),
                time,
                *key,
                wl_keyboard::KeyState::Released,
            );
        }
    }
}

fn deliver_modifiers<D>(state: &mut D, seat: &Seat<D>, id: ObjectId)
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
    D::KeyboardFocus: WaylandFocus + Clone,
{
    let Some(keyboard) = seat.get_keyboard() else {
        return;
    };
    let Some((focus, client)) = focused_client(&keyboard) else {
        return;
    };
    // A keymap change carries the modifiers with it.
    if send_keymap(state, seat, &keyboard, id.clone(), &focus, &client) {
        return;
    }
    let Some(mods) = state.virtual_keyboard_bindings().mods(&id) else {
        return;
    };
    focus.modifiers(seat, state, mods, SERIAL_COUNTER.next_serial());
    record_virtual_mods(state, &keyboard, mods);
}

/// Put the seat's keymap and modifiers back on every `wl_keyboard` a virtual
/// keyboard moved off them. Call before forwarding a physical key: smithay only
/// re-sends a keymap it knows it changed, and it does not know about these.
/// The modifiers go back even when no keymap does (see `virtual_mods_sent`).
pub fn restore_seat_keymap<D>(state: &mut D, seat: &Seat<D>)
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
    D::KeyboardFocus: WaylandFocus + Clone,
{
    let bindings = state.virtual_keyboard_bindings();
    if bindings.foreign_keymaps.is_empty() && !bindings.virtual_mods_sent {
        return;
    }
    let Some(keyboard) = seat.get_keyboard() else {
        return;
    };
    let seat_text = seat_keymap(state, &keyboard);
    let records = std::mem::take(&mut state.virtual_keyboard_bindings().foreign_keymaps);
    state.virtual_keyboard_bindings().virtual_mods_sent = false;
    let held: Vec<WlKeyboard> = records
        .into_iter()
        .filter(|(_, text)| **text != *seat_text)
        .filter_map(|(kbd, _)| kbd.upgrade().ok())
        .collect();
    let bindings = state.virtual_keyboard_bindings();
    if let Some(seat_keymap) = bindings.seat_keymap.as_ref() {
        for kbd in held {
            if let Err(err) = seat_keymap.file.send(&kbd) {
                tracing::warn!("virtual keyboard: failed to restore the seat keymap: {err}");
            }
        }
    }
    if let Some(focus) = keyboard.current_focus() {
        focus.modifiers(
            seat,
            state,
            keyboard.modifier_state(),
            SERIAL_COUNTER.next_serial(),
        );
    }
}

/// Call after the seat's keymap may have changed. smithay broadcasts a changed
/// keymap to every `wl_keyboard`, which overwrites whatever virtual keymap any
/// of them held, so the records go with it; one that compiles to the same text
/// reaches nobody and leaves them true.
pub fn seat_keymap_changed<D>(state: &mut D, keyboard: &KeyboardHandle<D>)
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
{
    let Some(old) = state.virtual_keyboard_bindings().seat_keymap.take() else {
        return;
    };
    if *seat_keymap(state, keyboard) != *old.text {
        state.virtual_keyboard_bindings().foreign_keymaps.clear();
    }
}

impl<D> GlobalDispatch2<ZwpVirtualKeyboardManagerV1, D> for VirtualKeyboardManagerGlobalData
where
    D: Dispatch<ZwpVirtualKeyboardManagerV1, VirtualKeyboardManagerUserData>,
    D: Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<D>>,
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, VirtualKeyboardManagerUserData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ZwpVirtualKeyboardManagerV1, D> for VirtualKeyboardManagerUserData
where
    D: Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<D>>,
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &ZwpVirtualKeyboardManagerV1,
        request: zwp_virtual_keyboard_manager_v1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        if let zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { seat, id } =
            request
        {
            let seat = Seat::<D>::from_resource(&seat).unwrap();
            let keyboard = data_init.init(id, VirtualKeyboardUserData { seat });
            state
                .virtual_keyboard_bindings()
                .keyboards
                .insert(keyboard.id(), VirtualKeyboard::default());
        }
    }
}

impl<D> Dispatch2<ZwpVirtualKeyboardV1, D> for VirtualKeyboardUserData<D>
where
    D: SeatHandler + VirtualKeyboardBindingHandler + 'static,
    D::KeyboardFocus: WaylandFocus + Clone,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &ZwpVirtualKeyboardV1,
        request: zwp_virtual_keyboard_v1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let id = resource.id();
        match request {
            zwp_virtual_keyboard_v1::Request::Keymap { format, fd, size } => {
                state
                    .virtual_keyboard_bindings()
                    .track_keymap(id, format, &fd, size as usize);
            }
            zwp_virtual_keyboard_v1::Request::Key {
                time,
                key,
                state: key_state,
            } => {
                if !state.virtual_keyboard_bindings().has_keymap(&id) {
                    resource.post_error(
                        zwp_virtual_keyboard_v1::Error::NoKeymap,
                        "`key` sent before keymap.",
                    );
                    return;
                }
                if handle_key(state, id.clone(), key, key_state) {
                    return;
                }
                deliver_key(state, &self.seat, id, time, key, key_state);
            }
            zwp_virtual_keyboard_v1::Request::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                if !state.virtual_keyboard_bindings().has_keymap(&id) {
                    resource.post_error(
                        zwp_virtual_keyboard_v1::Error::NoKeymap,
                        "`modifiers` sent before keymap.",
                    );
                    return;
                }
                state.virtual_keyboard_bindings().track_modifiers(
                    id.clone(),
                    mods_depressed,
                    mods_latched,
                    mods_locked,
                    group,
                );
                deliver_modifiers(state, &self.seat, id);
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &ZwpVirtualKeyboardV1) {
        let Some(kb) = state
            .virtual_keyboard_bindings()
            .keyboards
            .remove(&resource.id())
        else {
            return;
        };
        release_forwarded(&self.seat, &kb.forwarded);
        // Here rather than at the next physical key, which on a touch-only
        // device may never come.
        restore_seat_keymap(state, &self.seat);
    }
}

// The protocol handling above derives from smithay's
// `src/wayland/virtual_keyboard/`:
//
// Copyright (c) 2017 Victor Berger and Victoria Brekenfeld
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to
// deal in the Software without restriction, including without limitation the
// rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
// sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
// IN THE SOFTWARE.
