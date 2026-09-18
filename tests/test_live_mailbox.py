import datetime
import email
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("live_mailbox", ROOT / "scripts/live_mailbox.py")
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)

ENVIRONMENT = {"SHEP_LIVE_IMAP_HOST": "mail.example", "SHEP_LIVE_IMAP_PORT": "993",
               "SHEP_LIVE_IMAP_USER": "e2e@example", "SHEP_LIVE_IMAP_PASSWORD": "secret",
               "SHEP_LIVE_SMTP_HOST": "mail.example", "SHEP_LIVE_SMTP_PORT": "587"}


class LiveMailboxTests(unittest.TestCase):
    def test_settings_need_every_variable(self):
        settings = live.settings_from_environment(ENVIRONMENT)
        self.assertEqual((settings.host, settings.port, settings.user, settings.smtp_port), ("mail.example", 993, "e2e@example", 587))
        for name in live.VARIABLES:
            self.assertIsNone(live.settings_from_environment({**ENVIRONMENT, name: " "}))
            self.assertIsNone(live.settings_from_environment({k: v for k, v in ENVIRONMENT.items() if k != name}))

    def test_seed_is_deterministic_and_spread_over_days(self):
        messages = live.seed_messages()
        self.assertEqual(messages, live.seed_messages())
        self.assertEqual(len(messages), live.INBOX_COUNT + live.DELETED_COUNT)
        inbox = [m for m in messages if m.folder == "INBOX"]
        deleted = [m for m in messages if m.folder == live.DELETED_FOLDER]
        self.assertEqual((len(inbox), len(deleted)), (120, 4))
        self.assertEqual(len({m.message_id for m in messages}), len(messages))
        self.assertEqual(len({m.subject for m in messages}), len(messages))
        self.assertTrue(len({m.sender_address for m in inbox}) >= 4)
        unread = sum(m.unread for m in inbox)
        flagged = sum(m.flagged for m in inbox)
        self.assertTrue(0 < unread < len(inbox), unread)
        self.assertTrue(0 < flagged < len(inbox), flagged)
        stamps = [m.timestamp for m in inbox]
        self.assertEqual(stamps, sorted(stamps))
        self.assertGreater(stamps[-1] - stamps[0], datetime.timedelta(days=20))
        self.assertTrue(all(m.timestamp.tzinfo is not None for m in messages))
        self.assertEqual(live.expected_counts(messages), {"INBOX": {"total": 120, "unread": unread},
                                                          live.DELETED_FOLDER: {"total": 4, "unread": sum(m.unread for m in deleted)}})

    def test_rendered_message_parses_with_expected_headers_and_flags(self):
        message = live.seed_messages()[7]
        raw = live.render(message)
        self.assertNotIn(b"\n", raw.replace(b"\r\n", b""), "CRLF line ends only")
        parsed = email.message_from_bytes(raw)
        self.assertEqual(parsed["Subject"], message.subject)
        self.assertEqual(parsed["Message-ID"], message.message_id)
        self.assertIn(message.sender_address, parsed["From"])
        self.assertEqual(email.utils.parsedate_to_datetime(parsed["Date"]), message.timestamp)
        self.assertEqual(parsed.get_content_type(), "text/plain")
        self.assertIn(message.sender_name, parsed.get_payload())
        self.assertEqual(live.flags(message), "(\\Flagged)")
        self.assertEqual(live.flags(live.seed_messages()[0]), "(\\Seen \\Flagged)")
        self.assertEqual(live.flags(live.seed_messages()[1]), "()")

    def test_arrival_is_unread_and_newest(self):
        now = datetime.datetime(2026, 9, 18, 12, 0, tzinfo=datetime.timezone.utc)
        message = live.arrival(500, now)
        self.assertEqual((message.folder, message.unread, message.flagged), ("INBOX", True, False))
        self.assertEqual(message.subject, "Live arrival 500")
        self.assertEqual(message.timestamp, now)
        self.assertGreater(message.timestamp, max(m.timestamp for m in live.seed_messages()))
        self.assertEqual(live.flags(message), "()")

    def test_list_lines_parse_and_only_created_folders_are_removable(self):
        lines = [b'(\\HasNoChildren \\Trash) "/" "Deleted Items"',
                 b'(\\HasNoChildren) "/" INBOX',
                 b'(\\HasChildren) "/" Projects',
                 b'(\\HasNoChildren) "/" "Projects/Design"',
                 b'(\\HasNoChildren \\Junk) "/" "Junk Mail"',
                 b'(\\HasNoChildren \\Sent) "/" "Sent Items"',
                 b'(\\HasNoChildren \\Drafts) "/" Drafts',
                 b'(\\HasNoChildren \\Archive) "/" Archive',
                 b'(\\HasNoChildren) NIL "Quoted \\"name\\""']
        folders = [live.parse_list_line(line) for line in lines]
        self.assertEqual(folders[0], (frozenset({"\\HasNoChildren", "\\Trash"}), "/", "Deleted Items"))
        self.assertEqual(folders[1][2], "INBOX")
        self.assertEqual(folders[8], (frozenset({"\\HasNoChildren"}), None, 'Quoted "name"'))
        self.assertIsNone(live.parse_list_line(b"garbage"))
        self.assertEqual(live.deletion_order(folders), ["Projects/Design", "Archive", "Projects", 'Quoted "name"'])
        self.assertTrue(live.protected("INBOX", frozenset()))
        self.assertFalse(live.protected("Archive", frozenset({"\\Archive"})), "the app must create Archive afresh each run")
        self.assertEqual(live.parse_status(b'"INBOX" (MESSAGES 120 UNSEEN 80)'), {"total": 120, "unread": 80})
        self.assertEqual(live.quote('a "b" c\\d'), '"a \\"b\\" c\\\\d"')
        self.assertFalse(live.selectable(frozenset({"\\Noselect"})))


if __name__ == "__main__":
    unittest.main()
