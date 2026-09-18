#!/usr/bin/env python3
"""Seed, inspect and reset the disposable live e2e mailbox over IMAP.

Standard library only. Connection details come from the SHEP_LIVE_* variables;
the password is read from the environment and never printed or written out.
The mailbox is disposable and shared with later runs: every run wipes every
folder, removes folders earlier runs created and appends the same messages.
"""
import datetime
import email.utils
import imaplib
import os
import re
import ssl
import sys
from dataclasses import dataclass

VARIABLES = ("SHEP_LIVE_IMAP_HOST", "SHEP_LIVE_IMAP_PORT", "SHEP_LIVE_IMAP_USER",
             "SHEP_LIVE_IMAP_PASSWORD", "SHEP_LIVE_SMTP_HOST", "SHEP_LIVE_SMTP_PORT")
# Special-use folders stay; anything else was created by a run and goes.
PROTECTED_ATTRIBUTES = frozenset({"\\Trash", "\\Junk", "\\Sent", "\\Drafts"})
PROTECTED_NAMES = frozenset({"INBOX"})
DELETED_FOLDER = "Deleted Items"
INBOX_COUNT = 120
DELETED_COUNT = 4
BASE_TIME = datetime.datetime(2026, 8, 1, 9, 0, tzinfo=datetime.timezone.utc)
SPACING = datetime.timedelta(hours=7, minutes=13)
SENDERS = (("Maya Chen", "maya@formstudio.example"), ("Oliver Grant", "oliver@linear.example"),
           ("Sophie Williams", "sophie@example.com"), ("Priya Natarajan", "priya@brightpath.example"),
           ("Tomás Rivera", "tomas@harbour.example"), ("Hannah Okafor", "hannah@ledgerline.example"),
           ("Studio Updates", "updates@formstudio.example"), ("Ben Ashworth", "ben@example.org"))
TOPICS = ("Quarterly planning notes", "Invoice reminder", "Coffee next week?", "Design review follow-up",
          "Workspace digest", "Travel booking confirmed", "Photos from the launch", "Contract draft",
          "Standup summary", "Reading list", "Server maintenance window", "Team lunch")
LIST_LINE = re.compile(rb'\((?P<attributes>[^)]*)\)\s+(?P<delimiter>"[^"]*"|NIL)\s+(?P<name>"(?:[^"\\]|\\.)*"|\S+)')


@dataclass(frozen=True)
class Settings:
    host: str
    port: int
    user: str
    password: str
    smtp_host: str
    smtp_port: int


@dataclass(frozen=True)
class SeedMessage:
    ordinal: int
    folder: str
    sender_name: str
    sender_address: str
    subject: str
    body: str
    timestamp: datetime.datetime
    unread: bool
    flagged: bool

    @property
    def message_id(self):
        return f"<live-seed-{self.ordinal}@shep-e2e.invalid>"


def settings_from_environment(environ):
    """The live settings, or None when any variable is unset so callers can skip."""
    if any(not environ.get(name, "").strip() for name in VARIABLES):
        return None
    return Settings(host=environ["SHEP_LIVE_IMAP_HOST"].strip(), port=int(environ["SHEP_LIVE_IMAP_PORT"]),
                    user=environ["SHEP_LIVE_IMAP_USER"].strip(), password=environ["SHEP_LIVE_IMAP_PASSWORD"],
                    smtp_host=environ["SHEP_LIVE_SMTP_HOST"].strip(), smtp_port=int(environ["SHEP_LIVE_SMTP_PORT"]))


def seed_message(ordinal, folder, base=BASE_TIME):
    """One deterministic message; the ordinal alone decides every field."""
    name, address = SENDERS[ordinal % len(SENDERS)]
    topic = TOPICS[ordinal % len(TOPICS)]
    subject = f"{topic} {ordinal + 1}"
    body = (f"Hello,\n\n{topic} for item {ordinal + 1} from {name}.\n"
            f"This is seeded live e2e mail; it carries no personal data.\n\nRegards,\n{name}\n")
    return SeedMessage(ordinal=ordinal, folder=folder, sender_name=name, sender_address=address,
                       subject=subject, body=body, timestamp=base + SPACING * ordinal,
                       unread=ordinal % 3 != 0, flagged=ordinal % 7 == 0)


def inbox_messages(count=INBOX_COUNT, base=BASE_TIME):
    return [seed_message(ordinal, "INBOX", base) for ordinal in range(count)]


def deleted_messages(count=DELETED_COUNT, base=BASE_TIME, offset=INBOX_COUNT):
    return [seed_message(offset + ordinal, DELETED_FOLDER, base) for ordinal in range(count)]


def seed_messages():
    return inbox_messages() + deleted_messages()


def arrival(ordinal, now=None):
    """An unread message delivered after seeding, newest in INBOX."""
    now = now or datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
    message = seed_message(ordinal, "INBOX", now - SPACING * ordinal)
    return SeedMessage(**{**message.__dict__, "subject": f"Live arrival {ordinal}", "unread": True, "flagged": False})


def render(message):
    """RFC 5322 bytes with CRLF line ends, ready for APPEND."""
    lines = [f"From: {message.sender_name} <{message.sender_address}>",
             "To: Live e2e <e2e@shep.so>",
             f"Subject: {message.subject}",
             f"Date: {email.utils.format_datetime(message.timestamp)}",
             f"Message-ID: {message.message_id}",
             "MIME-Version: 1.0",
             "Content-Type: text/plain; charset=utf-8",
             "Content-Transfer-Encoding: 8bit",
             ""]
    return ("\r\n".join(lines) + "\r\n" + message.body.replace("\n", "\r\n")).encode("utf-8")


def flags(message):
    values = [] if message.unread else ["\\Seen"]
    if message.flagged:
        values.append("\\Flagged")
    return "(" + " ".join(values) + ")"


def parse_list_line(line):
    """(attributes, delimiter, name) from one LIST response line, or None."""
    match = LIST_LINE.match(line)
    if not match:
        return None
    attributes = frozenset(match["attributes"].decode().split())
    delimiter = None if match["delimiter"] == b"NIL" else match["delimiter"].decode().strip('"')
    name = match["name"].decode()
    if name.startswith('"'):
        name = re.sub(r'\\(.)', r'\1', name[1:-1])
    return attributes, delimiter, name


def protected(name, attributes):
    return name in PROTECTED_NAMES or bool(PROTECTED_ATTRIBUTES & attributes)


def deletion_order(folders):
    """Removable folders, children before parents so DELETE never hits a non-empty parent."""
    removable = [(name, delimiter) for attributes, delimiter, name in folders if not protected(name, attributes)]
    return [name for name, delimiter in sorted(removable, key=lambda f: (-(f[0].count(f[1] or "\0")), f[0]))]


def expected_counts(messages=None):
    """Folder totals and unread counts the seed should leave on the server."""
    counts = {}
    for message in messages or seed_messages():
        entry = counts.setdefault(message.folder, {"total": 0, "unread": 0})
        entry["total"] += 1
        entry["unread"] += int(message.unread)
    return counts


def parse_status(line):
    """{"total": n, "unread": n} from a STATUS (MESSAGES UNSEEN) response line."""
    values = dict(re.findall(rb"(MESSAGES|UNSEEN) (\d+)", line))
    return {"total": int(values.get(b"MESSAGES", 0)), "unread": int(values.get(b"UNSEEN", 0))}


class Mailbox:
    """The test's own IMAP connection: seeding, one extra delivery and read-only checks."""

    def __init__(self, settings):
        self.settings = settings
        self.imap = imaplib.IMAP4_SSL(settings.host, settings.port, ssl_context=ssl.create_default_context(), timeout=30)
        self.imap.login(settings.user, settings.password)

    def close(self):
        try:
            self.imap.logout()
        except (imaplib.IMAP4.error, OSError):
            pass

    def folders(self):
        status, lines = self.imap.list()
        if status != "OK":
            raise RuntimeError("LIST failed")
        parsed = [parse_list_line(line) for line in lines if isinstance(line, bytes)]
        return [entry for entry in parsed if entry]

    def folder_names(self):
        return [name for _, _, name in self.folders()]

    def has_folder(self, name):
        return name in self.folder_names()

    def counts(self, folder):
        status, lines = self.imap.status(quote(folder), "(MESSAGES UNSEEN)")
        if status != "OK":
            raise RuntimeError(f"STATUS {folder} failed")
        return parse_status(lines[0])

    def all_counts(self):
        return {name: self.counts(name) for attributes, _, name in self.folders() if selectable(attributes)}

    def subjects(self, folder):
        self.imap.select(quote(folder), readonly=True)
        status, data = self.imap.search(None, "ALL")
        if status != "OK":
            raise RuntimeError(f"SEARCH {folder} failed")
        ids = data[0].split()
        if not ids:
            return []
        status, parts = self.imap.fetch(b",".join(ids), "(BODY.PEEK[HEADER.FIELDS (SUBJECT)])")
        subjects = []
        for part in parts:
            if isinstance(part, tuple):
                header = email.message_from_bytes(part[1])
                subjects.append(str(header.get("Subject", "")).strip())
        return subjects

    def wipe(self):
        folders = self.folders()
        for attributes, _, name in folders:
            if not selectable(attributes):
                continue
            self.imap.select(quote(name))
            status, data = self.imap.search(None, "ALL")
            if status == "OK" and data[0].split():
                self.imap.store("1:*", "+FLAGS.SILENT", "(\\Deleted)")
                self.imap.expunge()
            self.imap.close()
        for name in deletion_order(folders):
            self.imap.delete(quote(name))

    def append(self, message):
        date = imaplib.Time2Internaldate(message.timestamp.timestamp())
        status, _ = self.imap.append(quote(message.folder), flags(message), date, render(message))
        if status != "OK":
            raise RuntimeError(f"APPEND to {message.folder} failed")

    def seed(self, messages=None):
        for message in messages or seed_messages():
            self.append(message)

    def reset(self):
        self.wipe()
        self.seed()
        return self.all_counts()

    def deliver(self, ordinal):
        """Deliver one more unread message now; returns its subject for the arrival check."""
        message = arrival(ordinal)
        self.append(message)
        return message.subject


def quote(name):
    return '"' + name.replace("\\", "\\\\").replace('"', '\\"') + '"'


def selectable(attributes):
    return not ({"\\Noselect", "\\NonExistent"} & attributes)


def main(argv):
    command = argv[1] if len(argv) > 1 else "status"
    if command not in ("seed", "wipe", "status"):
        print("usage: live_mailbox.py seed|wipe|status", file=sys.stderr)
        return 2
    settings = settings_from_environment(os.environ)
    if settings is None:
        print("Set " + ", ".join(VARIABLES) + " first.", file=sys.stderr)
        return 2
    mailbox = Mailbox(settings)
    try:
        if command == "seed":
            mailbox.reset()
        elif command == "wipe":
            mailbox.wipe()
        for folder, counts in sorted(mailbox.all_counts().items()):
            print(f"{folder}: {counts['total']} messages, {counts['unread']} unread")
    finally:
        mailbox.close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
