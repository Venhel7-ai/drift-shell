# To do

Open work that is not filed as an issue, grouped by area. Behaviour that reads
like a bug but is settled — what input hit-tests, what a configured size means,
the inert resize band — lives in [caveats.md](caveats.md); do not re-file it
here. Line numbers drift; re-verify on pickup. Profiling tooling:
[profiling.md](profiling.md).

## Pointer focus

- **One hit test, not two.** `focus_under` (`src/state/viewport_animation.rs`)
  is a second cascade that forks on the cached `pointer_over_layer` flag, omits
  `pinned_window_under`, and omits the fullscreen cull of Top/Bottom. It feeds
  `warp_pointer`'s grabbed branch and `cursor_over_surface`, which
  `new_constraint` uses to decide whether to refresh — for a pinned window it
  always answers "not over", and only the lock guard in the pull saves it. Make
  `focus_cascade` return a `PointerPick { under, over_layer, over_screen_space }`
  so the classification travels with the result, have `pointer_focus_under_pick`
  assign the fields, and delete `focus_under`.
- **The pull refreshes `wl_pointer`, not the tablet tool's own focus.** A window
  closing or moving under a *resting* pen leaves `tool.down` aimed at the tool's
  cached surface until the pen moves. Low; the pen's next event corrects it.
- **The hold's rect is predicted from the configured size**, but the settle that
  ends it reads the committed size. A client answering at a size other than the
  one configured (min/max constraints, aspect-ratio clients) has the hold
  evaluated against a rect it never occupies. Self-limiting; note only.

## Session lock

- **Keyboard reaches the lock surface before `locked` is sent.** `enter_locked`
  focuses the lock surface while `pending_confirmation` is still `Some`, and
  every lock-surface commit in `Locked` does too. On the backstop path (an
  output that never presents) the user types into a visible prompt for up to a
  second, and a password landing inside that second meets `invalid_unlock` on
  the locker's `unlock_and_destroy`. Input does not re-light panels while the
  confirmation is pending, so the backstop's blank lands and the window is
  bounded by `LOCK_CONFIRM_TIMEOUT`; what remains is the second itself. The
  only closer is handing focus over from `finish_lock_confirmation`, after
  `locker.lock()` — a prompt deaf for one refresh normally and for the backstop
  second on a broken output. The no-flash guarantee is not up for negotiation.
- **Immediate confirmation on dead-lock takeover.** The replacement path
  re-enters `Pending` and re-awaits surfaces and presents, up to a second of
  extra `locked` latency for the new locker; lock frames are already on every
  output, so confirming at once is sound. Only if the keyboard-handoff reason
  given in the takeover arm's comment is not load-bearing — the commit arm
  already hands a late surface the keyboard from `Locked`.

## Virtual keyboard

The vendored protocol goes away once driftwm bumps past Smithay's virtual
keyboard rework (upstream PR 2159, handler-based, routed through
`process_input_event`): both vendoring reasons — the un-wrappable blanket
dispatch, and compositor bindings from a virtual key — disappear. Port the
seat-keymap swap and its layout/caps/num save-restore; keep the text dedup.
Everything below is worth doing only if that bump is far off.

- **A failed restore loses the record.** `restore_seat_keymap` takes
  `foreign_keymaps` before `KeymapFile::send` can fail, so a client whose
  restore errors is stranded on the virtual keymap with no record left to retry.
  Re-push on `Err`.
- **`release_forwarded` releases into the current focus.** If focus moved
  between the press and the keyboard's destruction, the old client already got
  `leave` and the new one receives a release without a press. Inert for
  toolkits, but wrong; record the target `wl_keyboard` per forwarded key, or
  clear `forwarded` on focus change.
- **No idle activity from virtual keys.** `notify_activity` runs only on the
  physical path, so remote or scripted typing never resets the idle timer.
- **`virtual_key_binding` lacks the `layout_independent` fallback** the physical
  filter applies (`src/input/keyboard.rs`); an on-screen keyboard on a
  non-Latin layout misses Latin-named bindings.
- **Key state `2` (repeated) reads as a release** at the `Key` arm; treat it as
  neither, or as pressed.
- **`KeymapFormat::NoKeymap` is rejected**, then the client is fatally errored
  on its first key. Accepting it as "keycodes are in the seat's keymap" is what
  the upstream rework does.
- **Serials go backwards within one batch.** The key's serial is minted before
  `restore_seat_keymap` mints its own for the `modifiers` resync, so the client
  sees `modifiers(N+k)` then `key(N)`. Nothing requires monotonicity; free to
  fix by minting after the restore.
- **`held_keymap_count` filters dead weaks**, so the leak counter is blind to
  the one thing that accumulates — stale records for destroyed `wl_keyboard`s.
  Bounded in practice by the `retain` in `send_keymap`, but the diagnostic does
  not prove it.
- **Upstream: `Keymap::new_from_fd` mmaps `size` bytes with no fstat or cap**, so
  a client-supplied `size` past the backing file SIGBUSes the compositor on the
  first read. driftwm reads instead of mapping; worth an upstream issue since it
  survives the rework.

## Touch and tablet

- **A touch drag the compositor takes over never ends the app's touch point.**
  The xdg move request arrives after the hardware frame that settled the
  finger's `down`, and smithay's `cancel` skips settled slots, so
  `cancel_unframed_touch` at the move-request sites has nothing to revoke: the
  app keeps a live touch point until its slot is reused by a later `down`.
  Ending it properly means a synthetic `up` (or `cancel`) on that slot alone
  before the grab installs; smithay has no per-slot cancel, so the `up` is the
  candidate. Toolkits tolerate the stale point; a canvas app mid-stroke may
  not.
- **Upstream: a client binding tablet-v2 while the pen is already in proximity
  gets `proximity_in` and `frame` but no position** — smithay's late-bind path
  carries a literal TODO for axis, motion and button. The client knows it is in
  proximity but not where until the next pen event. Relevant to issue 280 if
  the app binds lazily on first contact.

## Next smithay bump

- **`remove_constraint` changes shape** (upstream `e3d461a0`): the handler takes
  a `ConstraintRemove` enum and runs outside the constraint closure. The
  signature at `src/handlers/mod.rs` breaks, and part of the rationale for the
  snapshot lint in `src/input/constraint.rs` becomes moot.
- **winit moves to `0.31.0-beta.3`** (upstream `73f2570c`); the beta.2 pin in
  `Cargo.lock` and its note in `Cargo.toml` go with it.

## Performance

The B-series perf push and the blur cluster it deferred (B5b, S1, B12) have
shipped. What remains is opportunistic — do only if a profile flags it.

- **Gigapixel-TIFF decoder pool** has no cancellation of stale in-flight
  decodes; blobs upload regardless of visibility and back up during fast pans
  (`src/render/tile_worker.rs`, `tile_chunks.rs`). Cancel unwanted requests,
  drop off-viewport responses, bound the queue.
- **Held repeatable key and the exec loading cursor mark every output dirty** at
  refresh rate (`src/backend/udev.rs`). Mark only the active/cursor output.
  Single-output-marginal; likely not worth it.
- **Latent frame spikes** (config-dependent): synchronous shader-chunk bakes
  mid-frame (`src/render/shader_chunks.rs` — pre-bake a margin ring, pool the
  FBO); gigapixel-TIFF tile uploads up to ~25 ms/frame on the render thread
  (`src/render/mod.rs` — time-budget, or upload after `queue_frame`); the shadow
  shader evaluates ERF quadrature over the full window+pad quad
  (`src/shaders/shadow.glsl` — early-out interior fragments).
- **Redundant composites in non-integer refresh:content beats.**
  `compose_frame` runs before the frame is queued and `post_render` runs
  unconditionally after it, outside the match that catches `EmptyFrame`. At
  ratios like 144 Hz/60 fps video a second client commit can land mid-cycle and
  force a full `compose_frame` that smithay then drops as `EmptyFrame` — GPU
  work with no page flip, plus a callback send. Bounded by the estimated-vblank
  timer and only during active rendering. Fix: skip the compose and the
  callback send on the `EmptyFrame` path. The VBlank handler's direct
  `render_frame` is *not* worth routing through the `render_if_needed` gate: it
  clears `frames_pending` and the estimated timer just above, so the gate's
  conditions already hold there.
- **Animations sampled at `Instant::now()`** rather than predicted presentation
  time — a small judder source.
- **VRR.** driftwm has none; `src/protocols/output_management.rs` carries a
  stub. On-demand VRR by window visibility is a gaming pass that comes after
  the feature itself.

## Correctness

- **Adopting a stand-in reads the stage position, not the in-flight visual.**
  `adopt_relaunched` takes `stage.position_of` — the destination — so adopting
  a stand-in that a neighbouring cluster shift pushed within the last few
  hundred ms teleports the departing chrome to the end of the slide in one
  frame. The dismiss half is fixed; adopt is not, because seeding only the fade
  leaves the two crossfade halves offset for its whole life (worse than the
  pop), and seeding the incoming window too means converting a deliberate hold
  — `from == target`, which holds the slot until the client acks — into a
  finite leg in exactly the window that hold exists to cover, plus reassembling
  the CSD bar offset by hand. Cosmetic, narrow, and not the one-liner it looks
  like.
- **The stale-frame guard reads *unacked* configures, so an early-acking client
  goes unguarded.** `owes_a_configured_size` (`src/state/resize.rs`) scans
  `pending_configures()`, which empties the moment a client acks — and toolkits
  routinely ack before they redraw, so the stale frames that follow read as a
  grow past the settled footprint and get the window relocated beside a
  neighbour. Every defence so far is per-path: the fit/fill/fullscreen exits
  survive only because they leave a `pending_recenter` that gates the reflow,
  and the relaunch adopt because it owes its stable snap rect until the client
  commits the size it configured. Comparing committed geometry against the
  *last sent* configure would cover the class at once and let both retire; not
  taken where it was found because every window in the compositor rides that
  comparison.
