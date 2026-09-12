import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from gnome_notifications import validate_notification


class GnomeNotificationTests(unittest.TestCase):
    def test_actual_source_must_belong_to_shep_and_remain_present(self):
        notification = {"app": "so.shep.Shep.desktop", "title": "Morgan",
                        "body": "New mail from the background"}
        validate_notification([notification], "details")
        for items in [[], [notification, notification], [{**notification, "app": "another.desktop"}]]:
            with self.assertRaises(AssertionError):
                validate_notification(items, "details")

    def test_private_mode_rejects_sender_and_subject_disclosure(self):
        private = {"app": "so.shep.Shep.desktop", "title": "New email",
                   "body": "You have a new message in your Inbox."}
        validate_notification([private], "private")
        for field, value in [("title", "Morgan"), ("body", "New mail from the background")]:
            with self.assertRaises(AssertionError):
                validate_notification([{**private, field: value}], "private")

    def test_muted_arrival_rejects_any_visible_notification(self):
        validate_notification([], "muted")
        with self.assertRaises(AssertionError):
            validate_notification([{"app": "so.shep.Shep.desktop"}], "muted")
