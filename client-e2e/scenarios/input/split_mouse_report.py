"""A mouse report split across reads never reaches the composer as keystrokes."""

from __future__ import annotations

from e2e.app_server.scenario import Timeline

mouse_tracking_seen = True

# A loaded terminal fragments a mouse report across writes; a read that ends on
# the report's ESC makes the client emit Escape and type the rest as characters.
# Each fragment is its own write, settled once with the text typed after it.
timeline: Timeline = [
    {"fragments": ("\x1b", "[<35;10;39M", "ok")},
    "\x1b[<35;12;39M",
    {"fragments": ("\x1b", "[<35;11;39M", " world")},
]

screen_contains = ("ok world",)
screen_excludes = ("[<",)
