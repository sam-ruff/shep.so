import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

export default class NotificationObserver {
    enable() {
        const path = GLib.getenv('SHEP_NOTIFICATION_OBSERVATION');
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 50, () => {
            const notifications = Main.messageTray.getSources().flatMap(source =>
                source.notifications.map(notification => ({
                    app: source.app?.get_id() ?? null,
                    title: notification.title,
                    body: notification.body,
                })));
            const state = {
                shell_ready: !Main.layoutManager._startingUp && global.stage.mapped,
                overview_visible: Main.overview.visible,
                notifications,
                banner: Main.messageTray._bannerBin.visible && Main.messageTray._banner !== null,
                centre_open: Main.panel.statusArea.dateMenu.menu.isOpen,
                centre_mapped: Main.panel.statusArea.dateMenu.menu.actor.mapped,
            };
            GLib.file_set_contents(path, JSON.stringify(state));
            return GLib.SOURCE_CONTINUE;
        });
    }

    disable() {
        if (this._timer)
            GLib.source_remove(this._timer);
        this._timer = 0;
    }
}
