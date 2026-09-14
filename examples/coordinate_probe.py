#!/usr/bin/env python3
"""GTK4 click calibration without mixing surface, window and widget coordinates.

Launch in your test desktop. Take a targeted screenshot, choose the green 10x10
square's center from its pixels, and click it. A matching `hit: true` is the
acceptance condition. Do not pass the square's widget-local (85,85) directly to
window-relative click: margins and decorations are part of the screenshot.

No physical input is synthesized. The probe reports only events delivered to
its own widget. Requires Python GI, GTK4 and Graphene (no cairo Python binding).
"""
import json
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
gi.require_version("Graphene", "1.0")
from gi.repository import Gdk, GLib, Graphene, Gtk


def main():
    window = Gtk.Window(title="CUL coordinate probe")
    window.set_default_size(400, 300)
    window.set_titlebar(Gtk.HeaderBar())
    area = Gtk.Fixed()
    area.set_margin_start(6)
    area.set_margin_top(6)
    window.set_child(area)
    target = Gtk.Box()
    target.set_size_request(10, 10)
    target.add_css_class("calibration-target")
    area.put(target, 80, 80)
    css = Gtk.CssProvider()
    css.load_from_data(b".calibration-target { background: #00ff00; }")
    Gtk.StyleContext.add_provider_for_display(
        Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def pressed(controller, count, x, y):
        # Gesture coordinates belong to the same Fixed that contains the target.
        # Gdk.Event.get_position(), in contrast, uses surface coordinates:
        # https://docs.gtk.org/gdk4/method.Event.get_position.html
        ok, point = area.compute_point(window, Graphene.Point().init(x, y))
        print(json.dumps({
            "widget": [x, y],
            "window": [point.x, point.y] if ok else None,
            "surface_to_window": list(window.get_surface_transform()),
            "hit": 80 <= x < 90 and 80 <= y < 90,
        }), flush=True)

    click = Gtk.GestureClick()
    click.connect("pressed", pressed)
    area.add_controller(click)
    window.present()
    loop = GLib.MainLoop()
    window.connect("close-request", lambda *args: loop.quit())
    loop.run()


if __name__ == "__main__":
    main()
