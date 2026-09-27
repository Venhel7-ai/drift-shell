# Fork IPC additions (development version)

Based on malbiruk/driftwm commit 352333a8fa1b22171492d4b71a54102045c9a19d,
modified 2026-09-27 for Drift Shell. Upstream documentation remains in
`driftwm-fork/docs/ipc.md`. Existing upstream requests are preserved.

This is an implementation subset, not the complete V3 compositor contract.

`{"ShellCamera":{"output":"DP-1","center":[0,0],"zoom":1.0,"anchored":true}}`
sets the named output's camera immediately, cancels pending flight and toggles
its runtime anchor guard. Fullscreen outputs reject the request. Coordinates
are viewport center, Y-up. It is not an animated setCamera implementation.

`{"ShellLayout":{"revision":1,"targets":[{"id":3,"x":0,"y":0,"w":800,"h":600}]}}`
validates all targets (including client size constraints) before sending
configures. Coordinates describe visual-frame center, Y-up. At most 512 targets,
no duplicate IDs. Responses retain upstream Ok/Err format. The revision is
currently tracked by shell-core only; compositor replay protection is not implemented.
Wayland clients may commit sizes asynchronously or refuse requested sizes;
acceptance of a configure is not a guarantee of the final committed geometry.

Window inventory adds `transient: bool` to let shell-core exclude dialogs.

Only the same user's local IPC can issue commands; this interface must not
be proxied to a network endpoint.
