use std::cmp::{max, min};
use std::collections::BTreeSet;

use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::{RectangleKind, RegionAttributes};

/// Decompose a `RegionAttributes` (set of additive/subtractive rects) into a
/// non-overlapping list of rects covering the same area.
pub fn region_to_non_overlapping_rects(
    region: &RegionAttributes,
    output: &mut Vec<Rectangle<i32, Logical>>,
) {
    output.clear();

    let ys = BTreeSet::from_iter(
        region
            .rects
            .iter()
            .flat_map(|(_, r)| [r.loc.y, r.loc.y + r.size.h]),
    );

    let mut ys = ys.into_iter();
    let Some(mut lo) = ys.next() else {
        return;
    };

    let mut spans = Vec::<(i32, i32)>::new();

    for hi in ys {
        spans.clear();

        'region: for (kind, r) in &region.rects {
            if hi <= r.loc.y || r.loc.y + r.size.h <= lo {
                continue;
            }

            let mut x1 = r.loc.x;
            let mut x2 = r.loc.x + r.size.w;
            if x1 == x2 {
                continue;
            }

            match *kind {
                RectangleKind::Add => {
                    for i in (0..spans.len()).rev() {
                        let (start, end) = spans[i];

                        if end < x1 {
                            spans.insert(i + 1, (x1, x2));
                            continue 'region;
                        }

                        if x2 < start {
                            continue;
                        }

                        spans.remove(i);
                        x1 = min(x1, start);
                        x2 = max(x2, end);
                    }

                    spans.insert(0, (x1, x2));
                }
                RectangleKind::Subtract => {
                    for i in (0..spans.len()).rev() {
                        let (start, end) = spans[i];

                        if end <= x1 {
                            continue 'region;
                        }

                        if x2 <= start {
                            continue;
                        }

                        spans.remove(i);
                        if x2 < end {
                            spans.insert(i, (x2, end));
                        }
                        if start < x1 {
                            spans.insert(i, (start, x1));
                        }
                    }
                }
            }
        }

        for (x1, x2) in spans.drain(..) {
            output.push(Rectangle::from_extremities((x1, lo), (x2, hi)));
        }

        lo = hi;
    }
}
