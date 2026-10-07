import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import Clutter from 'gi://Clutter';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';

const SERVICE_NAME = 'dev.avifenesh.ComputerUseLinux.WindowControl';
const OBJECT_PATH = '/dev/avifenesh/ComputerUseLinux/WindowControl';
const BACKEND = 'gnome-shell-extension';

const WINDOW_CONTROL_XML = `
<node>
  <interface name="${SERVICE_NAME}">
    <method name="ListWindows">
      <arg name="json" type="s" direction="out"/>
    </method>
    <method name="ActivateWindow">
      <arg name="window_id" type="t" direction="in"/>
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
    </method>
    <method name="MoveWindow">
      <arg name="window_id" type="t" direction="in"/>
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
    </method>
    <method name="ResizeWindow">
      <arg name="window_id" type="t" direction="in"/>
      <arg name="width" type="i" direction="in"/>
      <arg name="height" type="i" direction="in"/>
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
    </method>
    <method name="ShowIndicator">
      <arg name="event_json" type="s" direction="in"/>
      <arg name="age_ms" type="u" direction="in"/>
    </method>
    <method name="HideIndicator">
      <arg name="shown" type="b" direction="out"/>
    </method>
    <method name="GetIndicatorState">
      <arg name="json" type="s" direction="out"/>
    </method>
    <method name="GetMonitorLayout">
      <arg name="json" type="s" direction="out"/>
    </method>
  </interface>
</node>
`;

const WindowControlDBus = GObject.registerClass(
class WindowControlDBus extends GObject.Object {
    constructor() {
        super();

        this._indicator = new ActivityIndicator();
        this._dbusObject = Gio.DBusExportedObject.wrapJSObject(
            WINDOW_CONTROL_XML, this);
        this._dbusObject.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.DBus.session.own_name(
            SERVICE_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            () => log(`Linux Computer Use Window Control lost DBus name ${SERVICE_NAME}`));
    }

    destroy() {
        this._indicator.destroy();
        if (this._nameId) {
            Gio.DBus.session.unown_name(this._nameId);
            this._nameId = 0;
        }

        this._dbusObject?.unexport();
        this._dbusObject?.run_dispose();
        this._dbusObject = null;
    }

    ShowIndicatorAsync([json, ageMs], invocation) {
        try {
            if (json.length > 4096)
                throw new Error('Indicator event exceeds the datagram limit');
            this._indicator.show(JSON.parse(json), ageMs, invocation.get_sender());
            invocation.return_value(new GLib.Variant('()', []));
        } catch (error) {
            invocation.return_dbus_error(`${SERVICE_NAME}.InvalidIndicatorEvent`, error.message);
        }
    }

    HideIndicatorAsync(_params, invocation) {
        invocation.return_value(new GLib.Variant('(b)', [this._indicator.hide()]));
    }

    GetIndicatorStateAsync(_params, invocation) {
        this._returnJson(invocation, this._indicator.state());
    }

    ListWindowsAsync(_params, invocation) {
        this._returnJson(invocation, this._listWindows());
    }

    ActivateWindowAsync([windowId], invocation) {
        const requestedId = Number(windowId);
        const window = this._listMetaWindows().find(
            candidate => Number(candidate.get_id()) === requestedId);

        if (!window) {
            invocation.return_value(new GLib.Variant('(bs)', [
                false,
                `No window matched window_id ${requestedId}`,
            ]));
            return;
        }

        try {
            if (Main.overview.visible)
                Main.overview.hide();

            if (window.minimized && typeof window.unminimize === 'function')
                window.unminimize();

            Main.activateWindow(window, global.get_current_time());
            invocation.return_value(new GLib.Variant('(bs)', [
                true,
                `Activated window_id ${requestedId}`,
            ]));
        } catch (error) {
            invocation.return_value(new GLib.Variant('(bs)', [
                false,
                `Activation failed: ${error.message}`,
            ]));
        }
    }

    MoveWindowAsync([windowId, x, y], invocation) {
        this._withWindow(windowId, invocation, window => {
            // move_frame positions the frame rect (what list_windows reports).
            window.move_frame(true, x, y);
            return `Moved window_id ${Number(windowId)} to ${x},${y}`;
        });
    }

    ResizeWindowAsync([windowId, width, height], invocation) {
        this._withWindow(windowId, invocation, window => {
            if (width <= 0 || height <= 0)
                throw new Error(`invalid size ${width}x${height}`);
            // GNOME 49 removed Meta.Window.get_maximized() (use is_maximized())
            // and dropped the flags argument from unmaximize(). Support both
            // API generations: shell 45-48 (get_maximized + flags) and 49+.
            if (window.is_maximized?.())
                window.unmaximize();
            else if (window.get_maximized?.())
                window.unmaximize(Meta.MaximizeFlags.BOTH);
            const rect = window.get_frame_rect();
            window.move_resize_frame(true, rect.x, rect.y, width, height);
            return `Resized window_id ${Number(windowId)} to ${width}x${height}`;
        });
    }

    GetMonitorLayoutAsync(_params, invocation) {
        const monitors = Main.layoutManager.monitors.map(monitor => ({
            index: monitor.index,
            x: monitor.x,
            y: monitor.y,
            width: monitor.width,
            height: monitor.height,
            primary: monitor.index === Main.layoutManager.primaryIndex,
            scale: monitor.geometry_scale ?? 1,
        }));
        this._returnJson(invocation, monitors);
    }

    _withWindow(windowId, invocation, action) {
        const requestedId = Number(windowId);
        const window = this._listMetaWindows().find(
            candidate => Number(candidate.get_id()) === requestedId);

        if (!window) {
            invocation.return_value(new GLib.Variant('(bs)', [
                false,
                `No window matched window_id ${requestedId}`,
            ]));
            return;
        }

        try {
            const message = action(window);
            invocation.return_value(new GLib.Variant('(bs)', [
                true,
                message,
            ]));
        } catch (error) {
            invocation.return_value(new GLib.Variant('(bs)', [
                false,
                `Window operation failed: ${error.message}`,
            ]));
        }
    }

    _returnJson(invocation, value) {
        invocation.return_value(new GLib.Variant('(s)', [
            JSON.stringify(value),
        ]));
    }

    _listWindows() {
        return this._listMetaWindows()
            .map(window => this._windowInfo(window))
            .filter(window => window !== null);
    }

    _listMetaWindows() {
        return global.get_window_actors()
            .map(actor => actor.meta_window)
            .filter(window => window && !window.is_override_redirect?.())
            .filter(window => window.get_window_type?.() !== Meta.WindowType.DESKTOP);
    }

    _windowInfo(window) {
        if (!window)
            return null;

        const app = Shell.WindowTracker.get_default().get_window_app(window);
        const rect = window.get_frame_rect();
        const workspace = window.get_workspace?.();

        return {
            window_id: Number(window.get_id()),
            title: window.get_title?.() ?? null,
            app_id: app?.get_id?.() ?? null,
            wm_class: window.get_wm_class?.() ?? null,
            pid: window.get_pid?.() ?? null,
            bounds: rect ? {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            } : null,
            workspace: workspace?.index?.() ?? null,
            focused: global.display.focus_window === window && !Main.overview.visible,
            hidden: window.minimized ?? false,
            client_type: clientTypeName(window.get_client_type?.()),
            backend: BACKEND,
        };
    }
});

function clientTypeName(value) {
    if (value === undefined || value === null)
        return null;
    if (value === Meta.WindowClientType.WAYLAND)
        return 'wayland';
    if (value === Meta.WindowClientType.X11)
        return 'x11';
    return 'unknown';
}

// GNOME has no layer-shell. These actors share the same event and capture
// protocol through the native indicator helper. All input stays with the app.
class ActivityIndicator {
    constructor() {
        this._root = new St.Widget({reactive: false, can_focus: false,
            layout_manager: new Clutter.FixedLayout(), visible: false});
        const chrome = {trackFullscreen: false};
        if (Number(Config.PACKAGE_VERSION.split('.')[0]) < 50)
            chrome.affectsInputRegion = false;
        Main.layoutManager.addChrome(this._root, chrome);
        this._pill = new St.Label({reactive: false, can_focus: false,
            style: 'background-color: rgba(20, 23, 30, 0.96); color: #f6f7fa; ' +
                'border-radius: 24px; padding: 11px 18px; font-size: 14px; font-weight: 600;'});
        this._bubble = new St.Label({reactive: false, can_focus: false,
            style: 'background-color: rgba(20, 23, 30, 0.96); color: #f6f7fa; ' +
                'border-radius: 10px; padding: 8px 12px; font-size: 13px; font-family: monospace;'});
        this._cursor = new St.DrawingArea({width: 112, height: 112,
            reactive: false, can_focus: false});
        this._cursor.connect('repaint', () => this._drawCursor());
        this._pulse = new St.DrawingArea({width: 112, height: 112, reactive: false, can_focus: false, visible: false});
        this._pulse.set_pivot_point(0.5, 0.5);
        this._pulse.connect('repaint', () => {
            const cr = this._pulse.get_context();
            const [r, g, b] = this._color ?? [0.3, 0.6, 1];
            cr.arc(56, 56, 34, 0, Math.PI * 2);
            cr.setSourceRGBA(r, g, b, 1); cr.setLineWidth(2); cr.stroke(); cr.$dispose();
        });
        this._root.add_child(this._pulse);
        this._root.add_child(this._pill);
        this._root.add_child(this._bubble);
        this._root.add_child(this._cursor);
        this._edges = [];
        this._sources = new Set();
        this._hiddenAt = -Infinity;
        this._event = null;
        this._owner = null;
        this._point = null;
        this._monitorSignal = Main.layoutManager.connect('monitors-changed', () => {
            this._rebuildEdges();
            if (this._event)
                this._layout(this._event, false);
        });
        this._ownerSignal = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus',
            'org.freedesktop.DBus', 'NameOwnerChanged', '/org/freedesktop/DBus',
            null, Gio.DBusSignalFlags.NONE, (_connection, _sender, _path, _iface, _signal, params) => {
                const [name, , owner] = params.deep_unpack();
                if (name === this._owner && !owner)
                    this.hide();
            });
        this._rebuildEdges();
    }

    _now() { return GLib.get_monotonic_time() / 1000; }

    _later(ms, callback) {
        const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, Math.max(1, Math.ceil(ms)), () => {
            this._sources.delete(id);
            callback();
            return GLib.SOURCE_REMOVE;
        });
        this._sources.add(id);
    }

    _clearTimers() {
        for (const id of this._sources)
            GLib.Source.remove(id);
        this._sources.clear();
    }

    _rebuildEdges() {
        for (const edge of this._edges)
            edge.destroy();
        this._edges = [];
        this._root.set_size(global.stage.width, global.stage.height);
        for (const monitor of Main.layoutManager.monitors) {
            const {x, y, width, height} = monitor;
            for (const [index, [ex, ey, ew, eh]] of [
                [x, y, width, 26], [x, y + height - 26, width, 26],
                [x, y, 26, height], [x + width - 26, y, 26, height]].entries()) {
                const edge = new St.Widget({x: ex, y: ey, width: ew, height: eh,
                    reactive: false, can_focus: false,
                    style: this._edgeStyle(index)});
                edge._direction = index;
                this._root.insert_child_at_index(edge, 0);
                this._edges.push(edge);
            }
        }
    }

    _edgeStyle(index) {
        const [r, g, b] = (this._color ?? [0.3, 0.6, 1]).map(v => Math.round(v * 255));
        const color = `rgba(${r}, ${g}, ${b}, 0.35)`;
        const start = index === 1 || index === 3 ? 'transparent' : color;
        const end = index === 1 || index === 3 ? color : 'transparent';
        return `background-gradient-direction: ${index < 2 ? 'vertical' : 'horizontal'}; ` +
            `background-gradient-start: ${start}; background-gradient-end: ${end};`;
    }

    _clickPulse(point) {
        this._pulse.remove_all_transitions();
        this._pulse.set_position(point[0] - 56, point[1] - 56);
        this._pulse.set_scale(0.3, 0.3);
        this._pulse.opacity = 230;
        this._pulse.queue_repaint();
        this._pulse.show();
        this._pulse.ease({scale_x: 1, scale_y: 1, opacity: 0, duration: 450,
            mode: Clutter.AnimationMode.EASE_OUT_CUBIC, onComplete: () => this._pulse.hide()});
    }

    _logicalPoint(event) {
        if (!Number.isFinite(event.x) || !Number.isFinite(event.y))
            return this._point;
        const monitors = Main.layoutManager.monitors;
        if (!monitors.length)
            return null;
        const left = Math.min(...monitors.map(m => m.x));
        const top = Math.min(...monitors.map(m => m.y));
        const right = Math.max(...monitors.map(m => m.x + m.width));
        const bottom = Math.max(...monitors.map(m => m.y + m.height));
        const [width, height] = event.space ?? [0, 0];
        if (width > 0 && height > 0)
            return [left + event.x * (right - left) / width,
                top + event.y * (bottom - top) / height];
        return [event.x, event.y];
    }

    _layout(event, animate) {
        const point = this._logicalPoint(event);
        const monitor = Main.layoutManager.monitors.find(m => point &&
            point[0] >= m.x && point[0] < m.x + m.width &&
            point[1] >= m.y && point[1] < m.y + m.height) ?? Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        const pillWidth = Math.min(560, monitor.width - 32);
        this._pill.set_width(pillWidth);
        this._pill.set_position(monitor.x + (monitor.width - pillWidth) / 2, monitor.y + 34);
        if (point) {
            if (!this._point)
                this._cursor.set_position(point[0] + 34, point[1] + 64);
            this._cursor.remove_all_transitions();
            if (animate) {
                this._cursor.ease({x: point[0] - 56, y: point[1] - 56, duration: 320,
                    mode: Clutter.AnimationMode.EASE_OUT_CUBIC,
                    onComplete: () => {
                        if (Number.isFinite(event.x) && Number.isFinite(event.y) &&
                            (event.tool === 'click' || event.tool === 'drag'))
                            this._clickPulse(point);
                    }});
            } else {
                this._cursor.set_position(point[0] - 56, point[1] - 56);
            }
            this._cursor.show();
            this._bubble.set_position(
                Math.max(monitor.x + 8, Math.min(point[0] + 18, monitor.x + monitor.width - 460)),
                Math.max(monitor.y + 8, Math.min(point[1] + 22, monitor.y + monitor.height - 70)));
        } else {
            this._cursor.hide();
            this._bubble.set_position(monitor.x + (monitor.width - 460) / 2, monitor.y + 98);
        }
        this._point = point;
    }

    show(event, ageMs, owner) {
        if (!event || typeof event.tool !== 'string' || !event.tool)
            throw new Error('Indicator action must name a tool');
        if (event.space !== undefined && (!Array.isArray(event.space) || event.space.length !== 2 ||
            !event.space.every(n => Number.isFinite(n) && n > 0)))
            throw new Error('Indicator coordinate space must contain positive dimensions');
        if (event.keys !== undefined && (!Array.isArray(event.keys) ||
            !event.keys.every(key => typeof key === 'string')))
            throw new Error('Indicator keys must be strings');
        if (event.text !== undefined && typeof event.text !== 'string')
            throw new Error('Indicator text must be a string');
        for (const key of ['x', 'y']) {
            if (event[key] !== undefined && !Number.isFinite(event[key]))
                throw new Error('Indicator coordinates must be finite');
        }
        this._clearTimers();
        this._event = event;
        this._owner = owner;
        this._root.remove_all_transitions();
        this._root.opacity = 255;
        if (ageMs >= 8000) {
            this.hide();
            return;
        }
        const name = typeof event.agent === 'string' ? event.agent.slice(0, 96) : 'Agent';
        this._pill.text = `●  ${name} is using your computer · ${event.tool}`;
        this._bubble.text = Array.isArray(event.keys) && event.keys.length
            ? event.keys.join(' + ') : event.text ?? '';
        this._bubble.visible = !!this._bubble.text && ageMs < 2200;
        const colors = {claude: '#e08a67', codex: '#4c9dff', opencode: '#8bd47e',
            gemini: '#a8b4f8', antigravity: '#a8b4f8', hermes: '#caa2ff',
            pi: '#c08cf0', cursor: '#b7d1aa', goose: '#c5aa79'};
        const hex = colors[name.toLowerCase()] ?? '#9b8cff';
        this._color = [1, 3, 5].map(offset => parseInt(hex.slice(offset, offset + 2), 16) / 255);
        for (const edge of this._edges)
            edge.set_style(this._edgeStyle(edge._direction));
        this._layout(event, ageMs === 0);
        this._cursor.queue_repaint();
        this._root.show();
        if (this._bubble.visible)
            this._later(Math.max(1, 2200 - ageMs), () => this._bubble.hide());
        this._later(8000 - ageMs, () => {
            this._root.ease({opacity: 0, duration: 350,
                mode: Clutter.AnimationMode.EASE_OUT_QUAD, onComplete: () => this.hide()});
        });
    }

    _drawCursor() {
        const cr = this._cursor.get_context();
        if (!cr)
            return;
        const [r, g, b] = this._color ?? [0.3, 0.6, 1];
        cr.moveTo(56, 56);
        cr.lineTo(63, 77);
        cr.lineTo(68, 68);
        cr.lineTo(78, 64);
        cr.closePath();
        cr.setSourceRGBA(r, g, b, 1);
        cr.fillPreserve();
        cr.setSourceRGBA(1, 1, 1, 1);
        cr.setLineWidth(2);
        cr.stroke();
        cr.$dispose();
    }

    hide() {
        const shown = this._root.visible || this._now() - this._hiddenAt < 120;
        this._clearTimers();
        this._root.remove_all_transitions();
        this._cursor.remove_all_transitions();
        this._pulse.remove_all_transitions();
        this._pulse.hide();
        if (this._root.visible)
            this._hiddenAt = this._now();
        this._root.hide();
        return shown;
    }

    state() {
        return {visible: this._root.visible, opacity: this._root.opacity,
            reactive: this._root.reactive, can_focus: this._root.can_focus,
            agent: this._event?.agent ?? null, tool: this._event?.tool ?? null,
            text: this._bubble.text, point: this._point,
            pointer: global.get_pointer().slice(0, 2),
            focused_pid: global.display.focus_window?.get_pid() ?? null,
            overview: Main.overview.visible,
            color: this._color ?? null};
    }

    destroy() {
        this.hide();
        Main.layoutManager.disconnect(this._monitorSignal);
        Gio.DBus.session.signal_unsubscribe(this._ownerSignal);
        Main.layoutManager.removeChrome(this._root);
        this._root.destroy();
    }
}

export default class ComputerUseLinuxWindowControlExtension extends Extension {
    enable() {
        this._dbusServer = new WindowControlDBus();
    }

    disable() {
        this._dbusServer?.destroy();
        this._dbusServer = null;
    }
}
