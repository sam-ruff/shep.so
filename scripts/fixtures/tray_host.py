#!/usr/bin/python3
"""An owned Linux StatusNotifier host with native GTK menus for MCP tests.

Launch only on the harness's private bus/display. Actions are ordinary pointer
input into this host; menu entries and clicks use the real SNI/DBusMenu protocol.
"""
import argparse
import json
import os
from pathlib import Path
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk, Gdk, GdkPixbuf

SNI = "org.kde.StatusNotifierItem"
WATCHER = "org.kde.StatusNotifierWatcher"
MENU = "com.canonical.dbusmenu"
PROPERTIES = "org.freedesktop.DBus.Properties"


class Host(dbus.service.Object):
    def __init__(self, bus, output):
        self.bus = bus
        self.output = output
        self.item = None
        self.service = ""
        self.menu_path = ""
        self.menu_entries = []
        self.icon_name = ""
        self.icon_themed = False
        self.icon_sizes = []
        self.notifications = []
        self.notification_service = Notifications(bus, self)
        self.name = dbus.service.BusName(WATCHER, bus=bus, do_not_queue=True)
        super().__init__(self.name, "/StatusNotifierWatcher")
        self.window = Gtk.Window(title="Shep tray test host")
        Gtk.IconTheme.get_default().append_search_path(str(output.parent / "icon-theme"))
        Gtk.Settings.get_default().set_property("gtk-application-prefer-dark-theme", False)
        self.window.set_default_size(200, 110)
        self.window.set_resizable(False)
        self.button = Gtk.Button(label="Waiting for Shep")
        self.button.connect("clicked", self.open_menu)
        layout = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.button.set_size_request(200,48)
        self.theme_button = Gtk.Button(label="Switch system icon theme")
        self.theme_button.set_size_request(200,48)
        self.theme_button.connect("clicked", self.toggle_theme)
        layout.pack_start(self.button,True,True,0)
        layout.pack_start(self.theme_button,True,True,0)
        self.window.add(layout)
        self.window.connect("destroy", lambda *_: Gtk.main_quit())
        self.window.show_all()
        self.record()

    def record(self, **extra):
        value = {"service": self.service, "menu_path": self.menu_path,
                 "entries": self.menu_entries, "notifications": self.notifications,
                 "icon_name": self.icon_name, "icon_themed": self.icon_themed,
                 "icon_sizes": self.icon_sizes,
                 "dark": bool(Gtk.Settings.get_default().get_property("gtk-application-prefer-dark-theme")), **extra}
        pending = self.output.with_suffix(".tmp")
        pending.write_text(json.dumps(value))
        pending.replace(self.output)

    @dbus.service.method(WATCHER, in_signature="s", out_signature="", sender_keyword="sender")
    def RegisterStatusNotifierItem(self, service, sender=None):
        self.service = str(sender if str(service).startswith("/") else service)
        path = str(service) if str(service).startswith("/") else "/StatusNotifierItem"
        self.item = self.bus.get_object(self.service, path)
        GLib.idle_add(self.load_item)

    @dbus.service.method(WATCHER, in_signature="s", out_signature="")
    def RegisterStatusNotifierHost(self, _service):
        pass

    @dbus.service.method(PROPERTIES, in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return self.GetAll(interface)[name]

    @dbus.service.method(PROPERTIES, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        if interface != WATCHER:
            return {}
        return {"IsStatusNotifierHostRegistered": dbus.Boolean(True),
                "RegisteredStatusNotifierItems": dbus.Array([self.service] if self.service else [], signature="s"),
                "ProtocolVersion": dbus.Int32(0)}

    def load_item(self):
        try:
            props = dbus.Interface(self.item, PROPERTIES)
            self.menu_path = str(props.Get(SNI, "Menu"))
            self.icon_name = str(props.Get(SNI,"IconName"))
            self.icon_themed = Gtk.IconTheme.get_default().has_icon(self.icon_name)
            pixmaps = props.Get(SNI, "IconPixmap")
            self.icon_sizes = [int(width) for width, height, _raw in pixmaps if width == height]
            if self.icon_themed:
                icon = Gtk.Image.new_from_icon_name(self.icon_name, Gtk.IconSize.MENU)
                icon.set_pixel_size(22)
                self.button.set_image(icon)
                self.button.set_always_show_image(True)
            elif pixmaps:
                width, height, raw = pixmaps[0]
                rgba = bytearray(bytes(raw))
                for offset in range(0, len(rgba), 4):
                    a, r, g, b = rgba[offset:offset+4]
                    rgba[offset:offset+4] = bytes((r, g, b, a))
                data = GLib.Bytes.new(bytes(rgba))
                pixbuf = GdkPixbuf.Pixbuf.new_from_bytes(data, GdkPixbuf.Colorspace.RGB,
                    True, 8, width, height, width*4)
                self.button.set_image(Gtk.Image.new_from_pixbuf(pixbuf))
                self.button.set_always_show_image(True)
            self.button.set_label("Shep")
            self.record()
        except dbus.DBusException as error:
            self.record(error=str(error))
        return False

    def toggle_theme(self, _button):
        settings = Gtk.Settings.get_default()
        settings.set_property("gtk-application-prefer-dark-theme",
                              not settings.get_property("gtk-application-prefer-dark-theme"))
        self.record()

    def open_menu(self, _button):
        if self.item is None:
            return
        try:
            interface = dbus.Interface(self.bus.get_object(self.service, self.menu_path), MENU)
            _revision, layout = interface.GetLayout(0, -1, dbus.Array([], signature="s"))
            menu = Gtk.Menu()
            self.menu_entries = []
            for identifier, properties, _children in layout[2]:
                if properties.get("type") == "separator":
                    entry = Gtk.SeparatorMenuItem()
                else:
                    label = str(properties.get("label", ""))
                    self.menu_entries.append({"id":int(identifier), "label":label})
                    entry = Gtk.MenuItem(label=label)
                    entry.set_sensitive(bool(properties.get("enabled", True)))
                    entry.connect("activate", self.activate, interface, identifier)
                menu.append(entry)
            menu.show_all()
            menu.popup_at_widget(self.button, Gdk.Gravity.SOUTH_WEST,
                                 Gdk.Gravity.NORTH_WEST, Gtk.get_current_event())
            self.record()
        except dbus.DBusException as error:
            self.record(error=str(error))

    def activate(self, _item, interface, identifier):
        try:
            interface.Event(identifier, "clicked", dbus.Int32(0), dbus.UInt32(0))
            self.record(last_clicked=int(identifier))
        except dbus.DBusException as error:
            self.record(error=str(error))


class Notifications(dbus.service.Object):
    def __init__(self, bus, host):
        self.host = host
        self.name = dbus.service.BusName("org.freedesktop.Notifications", bus=bus, do_not_queue=True)
        super().__init__(self.name, "/org/freedesktop/Notifications")

    @dbus.service.method("org.freedesktop.Notifications", in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app, replaces, icon, title, body, actions, hints, timeout):
        self.host.notifications.append({"title": str(title), "body": str(body)})
        self.host.record()
        return dbus.UInt32(len(self.host.notifications))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", type=Path, required=True)
    args = parser.parse_args()
    expected = "unix:path="+str(args.state.parent / "tray-bus")
    if os.environ.get("DBUS_SESSION_BUS_ADDRESS") != expected:
        parser.error("The tray host requires its owned fixture bus")
    DBusGMainLoop(set_as_default=True)
    host = Host(dbus.SessionBus(), args.state)
    Gtk.main()
    host.remove_from_connection()


if __name__ == "__main__":
    main()
