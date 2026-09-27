//! The price of the pointer-focus pull, measured rather than argued:
//! `pull_cost_bench` prints what one `refresh_pointer_focus` costs per
//! iteration with the cursor over bare canvas (every arm of the cascade runs)
//! and over a window (the cheap path), next to one real absolute motion, which
//! runs the same cascade. The numbers live in `dev/docs/caveats.md`.

use std::time::Instant;

use smithay::utils::Point;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1;

use crate::state::StageWindow;

use super::input_backend::{FakeDevice, pointer_to_screen};
use super::{Fixture, map_top_layer, map_window, window_by_app_id};

#[test]
#[ignore = "prints timings for dev/docs/caveats.md; asserts nothing"]
fn pull_cost_bench() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let device = FakeDevice::mouse();

    // Twenty windows in a grid over the top-left of the viewport, leaving the
    // bottom-right corner bare. The fixture's camera starts at (-960, -540).
    for i in 0..20 {
        let app_id = format!("w{i}");
        map_window(&mut f, id, &app_id, (300, 200));
        let window = window_by_app_id(&mut f, &app_id).unwrap();
        let loc = Point::from((-960 + (i % 5) * 320, -540 + (i / 5) * 220));
        f.state()
            .map_window(StageWindow::Client(window), loc, false);
    }
    let pinned = window_by_app_id(&mut f, "w0").unwrap();
    f.state().pin_window(&pinned).unwrap();
    map_top_layer(
        &mut f,
        id,
        "bar",
        (1920, 40),
        Some(zwlr_layer_surface_v1::Anchor::Top),
    );
    map_top_layer(
        &mut f,
        id,
        "dock",
        (200, 100),
        Some(zwlr_layer_surface_v1::Anchor::Left),
    );
    f.double_roundtrip(id);

    // Runs on every CI pass: `--include-ignored` defeats the `#[ignore]`, and
    // a "soak" name would land it in the fd/RSS plateau run. Enough to average
    // out scheduler noise, not enough to be worth skipping; raise it locally
    // when re-measuring.
    let iterations = 1_000u32;
    let per_iteration = |f: &mut Fixture| {
        let start = Instant::now();
        for _ in 0..iterations {
            f.state().refresh_pointer_focus();
        }
        start.elapsed() / iterations
    };

    let bare = Point::from((1900.0, 1070.0));
    pointer_to_screen(&mut f, &device, bare);
    f.roundtrip(id);
    let pull_bare = per_iteration(&mut f);

    let start = Instant::now();
    for _ in 0..iterations {
        pointer_to_screen(&mut f, &device, bare);
    }
    let motion_bare = start.elapsed() / iterations;

    // Over the fourth window of the second row: not the pinned one, not
    // under either layer.
    pointer_to_screen(&mut f, &device, Point::from((1110.0, 320.0)));
    f.roundtrip(id);
    let pull_window = per_iteration(&mut f);

    eprintln!(
        "pull over bare canvas: {pull_bare:?}; pull over a window: {pull_window:?}; \
         one real absolute motion over bare canvas: {motion_bare:?}"
    );
}
