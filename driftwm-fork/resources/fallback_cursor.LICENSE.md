# Fallback cursor attribution

`fallback_cursor.rgba` is the 24×24 frame of the `default` (a.k.a. `left_ptr`)
cursor from the [elementary icon theme](https://github.com/elementary/icons),
extracted from `/usr/share/icons/elementary/cursors/default` (size 24,
hotspot 7×4) and converted to raw RGBA byte order.

The elementary icon theme is distributed under the **GNU General Public
License, version 3** (GPL-3.0), which is compatible with driftwm's GPL-3.0+
license. The original copyright belongs to the elementary contributors.

This asset is embedded into the driftwm binary and rendered only when no
xcursor theme can be resolved on the system (e.g. minimal installs without
any cursor theme on the xcursor search path), so the pointer remains
visible. As soon as a real cursor theme is found, the embedded fallback is
no longer used.
