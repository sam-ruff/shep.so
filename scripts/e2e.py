#!/usr/bin/env python3
"""Deterministic, non-AI equivalents of the native MCP interaction scenarios."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time
import unittest
import math

ROOT = Path(__file__).resolve().parents[1]


class McpClient:
    def __init__(self):
        self.process = subprocess.Popen([sys.executable, str(ROOT / "scripts/mcp_harness.py")],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.serial = 0
        self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": "shep-automated-e2e", "version": "1"}})
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def rpc(self, method, params=None):
        self.serial += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.serial, "method": method, "params": params or {}}) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("MCP server exited unexpectedly")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(response["error"])
        result = response["result"]
        if result.get("isError"):
            raise AssertionError(result["content"][0]["text"])
        return result

    def call(self, name, **arguments):
        result = self.rpc("tools/call", {"name": name, "arguments": arguments})
        return result.get("structuredContent", result)

    def batch(self, *actions):
        return self.call("desktop.batch", actions=list(actions))

    def close(self):
        if self.process.poll() is None:
            self.call("desktop.stop")
            self.process.stdin.close()
            self.process.wait(timeout=10)
            self.process.stdout.close()


def click(x, y): return {"type": "click", "x": x, "y": y}
def double_click(x, y): return {"type": "double_click", "x": x, "y": y}
def drag(x, y, end_x, end_y): return {"type": "drag", "x": x, "y": y, "end_x": end_x, "end_y": end_y, "duration_ms": 200}
def key(value): return {"type": "key", "key": value}
def type_text(value): return {"type": "type", "text": value}
def wait(ms=150): return {"type": "wait", "ms": ms}
def check(path, value, op="eq"): return {"type": "wait_for", "path": path, "value": value, "op": op}
def shot(name): return {"type": "screenshot", "name": name}


class NativeFlows(unittest.TestCase):
    def setUp(self):
        self.mcp = McpClient()
        result = self.mcp.call("desktop.start")
        self.addCleanup(self.mcp.close)
        self.artifacts = Path(result["artifacts"])
        print(f"\nEvidence: {result['artifacts']}", flush=True)

    def test_read_unread_and_flags_show_immediately_during_slow_save(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        state = self.mcp.call("desktop.state")
        initial, starred = state["unread"], state["starred"]
        self.mcp.batch(click(740, 100), check("unread", not initial), check("mail_pending", 1),
                       shot("read-change-pending"), click(784, 100), check("starred", not starred),
                       click(740, 100), check("unread", initial), check("mail_pending", 1),
                       key("ctrl+r"), check("busy", "sync", "contains"),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, check("unread", initial), check("starred", not starred),
                       shot("read-and-flag-committed"),
                       click(740, 100), check("unread", not initial), {**check("mail_pending", 0), "timeout_ms": 5000},
                       click(416, 349), click(416, 247), check("unread", not initial))

    def test_failed_read_and_flag_restore_state_without_blocking_navigation(self):
        self.mcp.call("desktop.start", mail_actions="fail")
        state = self.mcp.call("desktop.state")
        initial, starred = state["unread"], state["starred"]
        self.mcp.batch(click(740, 100), check("unread", not initial), check("mail_pending", 1),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, check("unread", initial), check("notice", "restored", "contains"),
                       shot("failed-read-restored"), click(784, 100), check("starred", not starred),
                       click(420, 349), check("selected", "Your weekly workspace digest"),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, click(420, 247), check("starred", starred),
                       shot("failed-flag-restored"))

    def test_archive_hides_immediately_and_commits_while_other_mail_is_readable(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(click(652, 100), check("total", 119), check("mail_pending", 1),
                       check("selected", "Your weekly workspace digest"), shot("archive-pending-next-mail"),
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       click(84, 398), check("folder", "Archive"), check("total", 1),
                       check("selected", "A little more room to think"), shot("archive-committed"),
                       key("m"), check("dialog", "Move"), check("focused_input", "folder-search"),
                       type_text("inbox"), key("Return"), check("total", 0), check("mail_pending", 1),
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       click(84, 278), check("folder", "INBOX"), check("total", 120))

    def test_failed_archive_restores_source_without_changing_navigation(self):
        self.mcp.call("desktop.start", mail_actions="fail")
        self.mcp.batch(key("BackSpace"), check("total", 119), check("mail_pending", 1),
                       shot("archive-optimistic-before-failure"),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, check("total", 120),
                       check("notice", "restored to Inbox", "contains"), shot("archive-rollback"),
                       click(420, 246), click(652, 100), check("total", 119),
                       click(84, 398), check("folder", "Archive"),
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       check("folder", "Archive"), check("total", 0),
                       click(84, 278), check("folder", "INBOX"), check("total", 120))

    def test_context_read_and_inbox_flag_use_the_clicked_message_during_slow_save(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch({"type": "click", "x": 403, "y": 450, "button": 3},
                       check("context_subject", "Coffee next Thursday?"),
                       key("Down"), key("Down"), key("Down"), key("Return"),
                       check("mail_rows.2.unread", False), check("mail_pending", 1),
                       check("mail_rows.0.unread", True), shot("context-read-pending-correct-row"),
                       click(568, 425), check("mail_rows.2.starred", True),
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       click(420, 450), check("selected", "Coffee next Thursday?"),
                       check("unread", False), check("starred", True),
                       shot("context-read-row-flag-committed"))

    def test_shortcut_clear_primary_secondary_and_cancel_capture(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(645, 156),
                       check("settings_tab", "Shortcuts"), shot("shortcut-clear-controls"),
                       click(988, 350), check("shortcuts.Move", ""), check("preferences_saved", True),
                       click(1070, 350), key("alt+m"), check("shortcut_secondary.Move", "Alt+M"),
                       click(1157, 350), check("shortcut_secondary.Move", ""), check("preferences_saved", True),
                       click(905, 350), click(988, 350), key("q"), check("shortcuts.Move", ""),
                       key("ctrl+1"), check("tab", "Mail"), key("m"), wait(80), check("dialog", None),
                       key("ctrl+comma"), check("tab", "Preferences"), click(905, 350), key("m"),
                       check("shortcuts.Move", "M"), check("preferences_saved", True),
                       key("ctrl+1"), key("m"), check("dialog", "Move"), key("Escape"))

    def test_sidebar_resize_window_size_and_contacts_toast(self):
        self.mcp.batch(drag(222, 500, 310, 500), check("sidebar_width", 305, "gte"),
                       check("preferences_saved", True), check("saved_sidebar_width", 305, "gte"),
                       shot("resized-sidebar-light"),
                       {"type": "resize", "width": 1200, "height": 800}, check("window_size", [1200.0, 800.0]),
                       check("preferences_saved", True), check("saved_window_size.width", 1200.0),
                       key("ctrl+comma"), check("tab", "Preferences"), wait(80),
                       click(906, 156), check("settings_tab", "Contacts"), shot("contacts-section"),
                       click(590, 323), type_text("friend@example.com, alex@example.com"),
                       click(430, 380), check("contacts", ["friend@example.com", "alex@example.com"]),
                       check("saved_toast", True), check("preferences_saved", True), shot("contacts-saved-toast"),
                       click(1162, 756), check("saved_toast", False),
                       click(400, 156), check("settings_tab", "General"), click(693, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), shot("resized-sidebar-dark"))

    def test_context_menu_survives_mouse_release_and_sync_refresh(self):
        self.mcp.batch(key("ctrl+r"), check("busy", "sync", "contains"),
                       {"type": "click", "x": 403, "y": 450, "button": 3}, check("context_subject", "Coffee next Thursday?"),
                       wait(100), check("context_subject", "Coffee next Thursday?"), check("busy", []),
                       check("context_subject", "Coffee next Thursday?"), shot("context-after-background-refresh"),
                       click(485, 624), check("context_menu", None), check("starred", True),
                       {"type": "click", "x": 403, "y": 450, "button": 3}, check("context_subject", "Coffee next Thursday?"),
                       key("Escape"), check("context_menu", None),
                       {"type": "click", "x": 403, "y": 450, "button": 3}, check("context_subject", "Coffee next Thursday?"),
                       click(1380, 730), check("context_menu", None))

    def test_preferences_search_and_tooltip_options(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(1150, 88), type_text("tooltip"),
                       check("settings_matches", ["Tooltips"]), shot("settings-search-results"),
                       click(500, 289), check("settings_group", "Tooltips"), shot("tooltip-settings"),
                       click(288, 342), check("tooltips", False), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), {"type": "hover", "x": 651, "y": 100}, wait(550), shot("all-tooltips-disabled"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(288, 342), check("tooltips", True),
                       click(288, 379), check("shortcut_tooltips", False), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), {"type": "hover", "x": 651, "y": 100}, wait(550), shot("tooltip-without-shortcut"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(288, 379), check("shortcut_tooltips", True),
                       key("ctrl+1"), check("tab", "Mail"), {"type": "hover", "x": 651, "y": 100}, wait(550), shot("tooltip-primary-only"),
                       {"type": "hover", "x": 90, "y": 112}, wait(550), shot("no-labeled-control-tooltip"))
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(563, 366), check("dark", True),
                       click(650, 88), type_text("font"), check("settings_matches", ["Reading and layout"]), shot("settings-search-compact-dark"),
                       click(450, 289), check("settings_group", "Reading and layout"), shot("settings-font-search-destination"))

    def test_inbox_context_menu_targets_clicked_message(self):
        self.mcp.batch({"type": "click", "x": 403, "y": 450, "button": 3},
                       check("context_subject", "Coffee next Thursday?"), shot("inbox-context-light"),
                       click(485, 624), check("context_menu", None),
                       click(403, 450), check("selected", "Coffee next Thursday?"), check("starred", True),
                       {"type": "click", "x": 403, "y": 450, "button": 3}, check("context_subject", "Coffee next Thursday?"),
                       key("Escape"), check("context_menu", None),
                       key("shift+F10"), check("context_subject", "Coffee next Thursday?"),
                       key("Return"), check("full_reader", True), key("Escape"), check("full_reader", False),
                       {"type": "click", "x": 403, "y": 450, "button": 3}, check("context_subject", "Coffee next Thursday?"),
                       click(486, 664), check("dialog", "Move"), check("focused_input", "folder-search"),
                       type_text("Archive"), key("Return"), check("dialog", None), check("total", 119),
                       click(85, 398), check("folder", "Archive"), check("selected", "Coffee next Thursday?"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(690, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), key("shift+F10"),
                       check("context_subject", "Coffee next Thursday?"), shot("inbox-context-dark"),
                       click(1400, 700), check("context_menu", None))

    def test_inbox_context_compact_and_reply_target(self):
        self.mcp.call("desktop.stop")
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch({"type": "click", "x": 357, "y": 450, "button": 3},
                       check("context_subject", "Coffee next Thursday?"), shot("inbox-context-compact"),
                       click(442, 275), check("dialog", "Compose"), check("fields.to", "Sophie Williams <hello2@example.com>"),
                       check("fields.subject", "Re: Coffee next Thursday?"), shot("context-reply-target"),
                       key("Escape"), check("dialog", None),
                       key("ctrl+comma"), check("tab", "Preferences"), shot("contacts-tabs-compact"))

    def test_email_text_selection_and_copy(self):
        self.mcp.batch(check("reader_text_ready", True),
                       drag(650, 314, 714, 314), check("reader_selected_text", "Hey Alex", "contains"),
                       key("ctrl+c"), shot("selected-email-text"),
                       key("ctrl+k"), check("focused_input", "search"), key("ctrl+v"), check("query", "Hey Alex", "contains"),
                       key("ctrl+a"), key("BackSpace"), check("total", 120), key("Escape"),
                       double_click(420, 243), check("full_reader", True), check("reader_text_ready", True),
                       click(95, 310), key("ctrl+a"), check("reader_selected_text", "Design lead", "contains"),
                       key("ctrl+c"), shot("full-reader-selectable"), key("m"), check("dialog", "Move"),
                       key("Escape"), check("dialog", None), key("Escape"), check("full_reader", False),
                       key("ctrl+comma"), check("tab", "Preferences"), click(690, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), wait(80), drag(650, 314, 714, 314),
                       check("reader_selected_text", "Hey Alex", "contains"), shot("selected-email-text-dark"))

    def test_documentation_screenshots(self):
        self.mcp.batch(check("selected", "A little more room to think"),
                       {"type": "hover", "x": 1430, "y": 910}, wait(400), shot("docs-mail-light"),
                       key("ctrl+comma"), check("tab", "Preferences"),
                       click(690, 366), check("dark", True),
                       key("ctrl+2"), check("tab", "Calendar"),
                       {"type": "hover", "x": 1430, "y": 910}, wait(400), shot("docs-calendar-dark"))

    def test_layout_gallery(self):
        self.mcp.batch(shot("compact-mail-header"), key("ctrl+comma"), check("tab", "Preferences"), shot("preferences-general"),
                       click(645, 156), check("settings_tab", "Shortcuts"), shot("shortcuts-layout"),
                       key("ctrl+1"), check("tab", "Mail"), key("c"), check("dialog", "Compose"), shot("compose-layout"), key("Escape"), check("dialog", None),
                       key("m"), check("dialog", "Move"), shot("move-layout"), key("Escape"), check("dialog", None),
                       key("ctrl+2"), check("tab", "Calendar"), click(1340, 45), check("dialog", "Event"), shot("event-layout"), key("Escape"), check("dialog", None),
                       key("ctrl+1"), check("tab", "Mail"), wait(80), key("ctrl+k"), check("focused_input", "search"), type_text("prototype"), check("total", 1), key("Escape"), check("dialog", None),
                       check("reply_count", 1), check("attachment_count", 4), shot("reply-layout"))

    def test_read_search_preload_and_mouse_navigation(self):
        self.mcp.batch(check("selected", "A little more room to think"), check("cache_entries", 3, "gte"),
                       check("page_prefetched", True), shot("mail-light"),
                       click(403, 450), check("selected", "Coffee next Thursday?"),
                       key("ctrl+k"), check("focused_input", "search"), type_text("prototype"), check("total", 1),
                       check("selected", "Re: A few thoughts on the prototype"), shot("search-results"),
                       key("ctrl+a"), type_text("no-match-938481"), check("total", 0),
                       key("ctrl+a"), key("BackSpace"), check("total", 120), key("Escape"))

    def test_move_mouse_and_keyboard_and_typing_protection(self):
        self.mcp.batch(key("m"), check("dialog", "Move"), shot("move-dialog"), key("Escape"), check("dialog", None),
                       key("ctrl+k"), check("focused_input", "search"), type_text("m"), check("query", "m"), check("dialog", None), key("ctrl+a"), key("BackSpace"), check("query", ""),
                       check("total", 120), check("selected", "A little more room to think"), key("Escape"), wait(80), key("m"), check("dialog", "Move"),
                       click(600, 407), check("dialog", None), check("total", 119),
                       click(85, 398), check("folder", "Archive"), check("total", 1), shot("archived-message"))

    def test_appearance_toggle_and_calendar_with_mouse(self):
        self.mcp.batch(click(187, 867), check("tab", "Preferences"), shot("preferences-light"),
                       click(690, 366), check("dark", True), shot("preferences-dark"),
                       click(90, 159), check("tab", "Calendar"), shot("calendar-dark"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(399, 366), check("dark", False),
                       click(90, 159), check("tab", "Calendar"), shot("calendar-light"))

    def test_remapping_persists_and_works(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(645, 156),
                       check("settings_tab", "Shortcuts"), click(946, 350), key("alt+m"),
                       check("shortcuts.Move", "Alt+M"), shot("remapped-shortcut"),
                       key("ctrl+1"), check("tab", "Mail"), key("m"), wait(), check("dialog", None),
                       key("alt+m"), check("dialog", "Move"), key("Escape"))

    def test_delete_archive_defaults_and_mail_returns_to_inbox(self):
        self.mcp.batch(check("shortcuts.Delete", "Mod+D"), check("shortcuts.Archive", "Backspace"),
                       check("shortcut_secondary.Archive", "Delete"),
                       key("BackSpace"), check("total", 119), key("Delete"), check("total", 118),
                       key("ctrl+d"), check("total", 117),
                       click(85, 398), check("folder", "Archive"), check("total", 2),
                       click(85, 115), check("folder", "INBOX"), check("total", 117),
                       key("ctrl+k"), check("focused_input", "search"), type_text("invoice"), check("query", "invoice"), check("total", 1),
                       key("ctrl+d"), check("folder", "INBOX"), check("query", "invoice"), check("total", 1),
                       key("ctrl+a"), key("BackSpace"), check("total", 117), key("Escape"),
                       click(85, 438), check("folder", "Trash"), check("total", 1), shot("trash-shortcut-result"),
                       click(85, 115), check("folder", "INBOX"), check("total", 117),
                       key("ctrl+comma"), check("tab", "Preferences"), click(286, 737), check("unified", False),
                       key("ctrl+1"), check("tab", "Mail"), click(85, 358), check("folder", "Archive"),
                       click(85, 115), check("folder", "INBOX"), check("account", "preview-work"), shot("account-inbox-unread-count"))

    def test_sidebar_inbox_shortcut_and_highlighted_return_move(self):
        self.mcp.call("desktop.start", long_folders=True)
        self.mcp.batch(click(100, 537), check("folder", "Projects"), check("sidebar_focus", True),
                       key("i"), check("folder", "INBOX"),
                       click(100, 537), check("folder", "Projects"), click(420, 244), check("sidebar_focus", False),
                       key("i"), check("folder", "Projects"), key("m"), check("dialog", "Move"), check("focused_input", "folder-search"),
                       type_text("inbox"), check("move_enter_destination", "INBOX"), shot("move-inbox-enter-highlight"),
                       key("Return"), check("dialog", None), check("total", 0),
                       click(85, 115), check("folder", "INBOX"), check("total", 121),
                       key("ctrl+comma"), check("tab", "Preferences"), click(645, 156), check("settings_tab", "Shortcuts"),
                       {"type": "hover", "x": 1200, "y": 700}, {"type": "scroll", "amount": 30}, wait(150), shot("sidebar-inbox-key-settings"),
                       click(988, 738), check("shortcuts.Inbox", ""), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), click(100, 537), check("folder", "Projects"),
                       key("i"), wait(80), check("folder", "Projects"),
                       key("ctrl+comma"), check("tab", "Preferences"),
                       {"type": "hover", "x": 1200, "y": 700}, {"type": "scroll", "amount": 30}, wait(120),
                       click(920, 738), key("alt+i"), check("shortcuts.Inbox", "Alt+I"), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), click(100, 537), check("folder", "Projects"), key("alt+i"), check("folder", "INBOX"))

    def test_secondary_shortcut_remap_conflict_and_disable(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(645, 156),
                       check("settings_tab", "Shortcuts"), shot("two-shortcut-slots"),
                       click(1100, 350), key("Delete"), check("notice", "assigned more than once", "contains"),
                       key("alt+m"), check("shortcut_secondary.Move", "Alt+M"), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), key("alt+m"), check("dialog", "Move"), key("Escape"),
                       key("m"), check("dialog", "Move"), key("Escape"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(645, 156),
                       click(1157, 353), check("shortcut_secondary.Move", ""), check("preferences_saved", True),
                       shot("secondary-shortcut-disabled"))
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(563, 366), check("dark", True),
                       click(620, 156), check("settings_tab", "Shortcuts"), shot("shortcuts-compact-dark"),
                       {"type": "hover", "x": 800, "y": 500}, {"type": "scroll", "amount": 20}, wait(120), shot("shortcuts-compact-trash-default"))

    def test_preferences_and_resize_keep_latest_changes(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"),
                       click(690, 366), check("dark", True),
                       click(286, 737), check("unified", False),
                       click(286, 773), check("cross_account_moves", True),
                       key("ctrl+1"), check("tab", "Mail"), wait(80),
                       drag(616, 500, 785, 500), check("reader_split", .44, "gte"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(399, 366), check("dark", False),
                       check("preferences_saved", True), check("saved_appearance", "Light"),
                       check("saved_reader_split", .44, "gte"), check("unified", False),
                       check("cross_account_moves", True), shot("latest-preferences-saved"),
                       key("ctrl+1"), check("tab", "Mail"), check("reader_split", .44, "gte"),
                       shot("latest-resize-preserved"))

    def test_compose_save_and_reopen_draft(self):
        self.mcp.batch(click(101, 214), check("dialog", "Compose"), shot("compose"))
        # Actual typing, including M, must remain in the input field.
        self.mcp.batch(click(654, 312), type_text("friend@example.com"),
                       click(650, 362), type_text("Meet me Monday"), check("dialog", "Compose"),
                       click(650, 485), type_text("A message written with the mouse and keyboard."),
                       click(990, 720), check("draft_count", 1), check("dialog", None), shot("saved-draft"),
                       click(98, 478), check("dialog", "Compose"),
                       check("fields.to", "friend@example.com"), check("fields.subject", "Meet me Monday"),
                       check("editor", "A message written with the mouse and keyboard.", "contains"), shot("reopened-draft"))

    def test_compose_autosaves_and_move_accepts_typed_folder(self):
        self.mcp.batch(key("c"), check("dialog", "Compose"),
                       click(650, 362), type_text("Autosaved thought"),
                       check("draft_count", 1), key("Escape"), check("dialog", None),
                       key("m"), check("dialog", "Move"), check("focused_input", "folder-search"), type_text("Archive"), key("Return"),
                       check("dialog", None), check("total", 119), shot("keyboard-move-complete"))

    def test_compose_recipients_and_native_file_picker(self):
        fixture = self.artifacts / "planning notes.txt"
        fixture.write_text("These exact bytes must survive reopening the draft.")
        self.mcp.batch(key("c"), check("dialog", "Compose"), wait(80),
                       click(650, 312), type_text("friend@example.com"), click(1003, 312), wait(80), shot("compose-recipients"),
                       click(650, 312), type_text("copy@example.com"), click(650, 361), type_text("hidden@example.com"),
                       click(650, 411), type_text("Planning with attachments"),
                       click(650, 470), type_text("Please read the attached notes."),
                       click(583, 769), {"type": "choose_file", "path": str(fixture)},
                       check("draft_io", False), check("draft_attachments.0.name", fixture.name), wait(400), shot("compose-attached-file"))
        second = self.artifacts / "review checklist with a long name.txt"
        third = self.artifacts / "project reference materials.bin"
        second.write_text("Second attachment")
        third.write_bytes(bytes([0, 255, 1, 128]))
        self.mcp.batch(click(583, 790), {"type": "choose_file", "path": str(second)},
                       check("draft_io", False), check("draft_attachments.1.name", second.name), wait(400),
                       click(583, 790), {"type": "choose_file", "path": str(third)},
                       check("draft_io", False), check("draft_attachments.2.name", third.name), wait(400), shot("compose-wrapped-attachments"))
        self.mcp.batch(click(575, 726), check("draft_io", False), check("draft_attachments.0.name", second.name),
                       check("draft_attachments.1.name", third.name), wait(150), click(990, 790), check("dialog", None))
        fixture.unlink(); second.unlink(); third.unlink()
        self.mcp.batch(click(98, 478), check("dialog", "Compose"), check("fields.cc", "copy@example.com"),
                       check("fields.bcc", "hidden@example.com"), check("draft_attachments.0.name", second.name),
                       check("editor", "Please read the attached notes.", "contains"), shot("reopened-attachments"),
                       click(465, 790), check("notice", "Sending is disabled in preview", "contains"),
                       check("dialog", "Compose"), check("draft_attachments.1.name", third.name), shot("send-failure-keeps-draft"),
                       key("Escape"), check("dialog", None), key("ctrl+comma"), check("tab", "Preferences"),
                       click(690, 366), check("dark", True), key("ctrl+1"), check("tab", "Mail"),
                       click(98, 478), check("dialog", "Compose"), shot("composer-dark-attachments"),
                       click(583, 790), {"type": "choose_file"}, check("draft_io", False),
                       check("draft_attachments.1.name", third.name), key("Escape"), check("dialog", None))


    def test_reply_all_mouse_and_remappable_shortcut(self):
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("prototype"),
                       check("total", 1), key("Escape"), wait(80), click(766, 830), check("dialog", "Compose"),
                       check("fields.to", "Daniel Park <team@example.com>, colleague@example.com"),
                       check("fields.cc", "copy@example.com"), check("fields.bcc", ""),
                       check("draft_in_reply_to", "<prototype@example.com>"), shot("reply-all-mouse"),
                       key("Escape"), check("dialog", None), key("r"), check("dialog", "Compose"),
                       check("fields.to", "Daniel Park <team@example.com>"), check("fields.cc", ""),
                       key("Escape"), check("dialog", None), key("shift+r"), check("dialog", "Compose"),
                       check("fields.cc", "copy@example.com"), shot("reply-all-keyboard"))

    def test_compact_composer_layout(self):
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch(key("c"), check("dialog", "Compose"), shot("compose-compact"),
                       click(450, 207), type_text("friend@example.com"), click(733, 207), wait(80),
                       click(450, 257), type_text("copy@example.com"), click(450, 306), type_text("hidden@example.com"),
                       click(450, 355), type_text("Compact composer"), click(450, 400), type_text("Room to write."),
                       shot("compose-compact-recipients"), click(720, 548), check("dialog", None), check("draft_count", 1))

    def test_conversation_reader_keeps_messages_separate(self):
        self.mcp.call("desktop.start", conversation_mail=True)
        self.mcp.batch(check("selected", "Re: Launch schedule"), check("conversation_total", 3),
                       check("loaded_message_id", "preview-work:INBOX:launch-2"), wait(150), shot("conversation-overview"),
                       click(800, 344), check("loaded_message_id", "preview-work:Sent:launch-1"),
                       check("selected_id", "preview-work:INBOX:launch-2"), check("attachment_count", 1), shot("conversation-sent-message"),
                       key("r"), check("dialog", "Compose"), check("fields.to", "maya@example.com"),
                       check("draft_in_reply_to", "<launch-1@example.com>"), shot("conversation-reply-target"),
                       key("Escape"), check("dialog", None), click(1366, 343),
                       check("conversation_rows.1.starred", True), check("starred", True),
                       check("loaded_message_id", "preview-work:Sent:launch-1"), shot("conversation-flagged-message"),
                       key("m"), check("dialog", "Move"), check("focused_input", "folder-search"),
                       type_text("Projects"), key("Return"), check("dialog", None),
                       check("conversation_rows.1.folder", "Projects"), check("loaded_message_id", "preview-work:INBOX:launch-2"),
                       click(1322, 429), check("conversation_collapsed", True), shot("conversation-collapsed"),
                       click(800, 430), check("conversation_collapsed", False),
                       key("ctrl+comma"), check("tab", "Preferences"), click(286, 809), check("group_conversations", False),
                       key("ctrl+1"), check("tab", "Mail"), check("loaded_message_id", "preview-work:INBOX:launch-2"), shot("individual-message-reading"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(286, 809), check("group_conversations", True),
                       click(690, 366), check("dark", True), key("ctrl+1"), check("tab", "Mail"),
                       check("conversation_total", 3), shot("conversation-dark"),
                       double_click(400, 244), check("full_reader", True), shot("conversation-full-reader"),
                       key("Escape"), check("full_reader", False))

    def test_conversation_paging(self):
        self.mcp.call("desktop.start", conversation_mail=True)
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("Long project review"),
                       check("total", 1), key("Escape"), check("conversation_total", 25),
                       check("conversation_offset", 20), wait(150), shot("conversation-latest-page"),
                       click(1288, 194), check("conversation_offset", 0), check("loaded_message_id", "preview-work:Projects:long-0"),
                       check("conversation_rows.19.remote_id", "long-19"), shot("conversation-first-page"),
                       click(1377, 194), check("conversation_offset", 20), check("loaded_message_id", "preview-work:Projects:long-20"),
                       check("conversation_rows.4.remote_id", "long-24"), shot("conversation-later-page"),
                       click(1400, 36), check("busy", "sync", "contains"),
                       check("loaded_message_id", "preview-work:Projects:long-20"), shot("conversation-during-sync"))

    def test_conversation_compact_layout(self):
        self.mcp.call("desktop.start", width=900, height=640, conversation_mail=True)
        self.mcp.batch(check("conversation_total", 3), check("loaded_message_id", "preview-work:INBOX:launch-2"),
                       wait(150), shot("conversation-compact"))

    def test_outbox_delivery_review_and_copy_recovery(self):
        self.mcp.call("desktop.start", outgoing_mail=True)
        self.mcp.batch(check("outgoing_pending", 2), click(91, 478), check("dialog", "Outbox"),
                       check("outgoing_rows.0.subject", "Delivery needs review"),
                       check("outgoing_rows.0.delivery", "Uncertain"), shot("outbox-delivery-review-light"),
                       click(650, 600), check("outgoing_pending", 2), check("outgoing_confirmed", False),
                       click(538, 522), check("notice", "Checking server copies is disabled in preview", "contains"),
                       check("busy", []), wait(150), shot("outbox-after-server-check"),
                       click(482, 532), check("outgoing_confirmed", True), click(650, 570),
                       check("outgoing_pending", 1), check("outgoing_rows.0.subject", "Sent copy needs review"),
                       check("outgoing_confirmed", False), check("busy", []), wait(150), shot("outbox-copy-recovery-light"),
                       click(671, 614), check("outgoing_confirmed", False), check("outgoing_pending", 1),
                       click(482, 574), check("outgoing_confirmed", True), click(671, 614),
                       check("notice", "disabled in preview", "contains"), check("busy", []),
                       check("outgoing_pending", 1), wait(150), shot("outbox-copy-retry-error"),
                       click(799, 584), check("outgoing_pending", 0), check("outgoing_rows", []),
                       shot("outbox-empty-light"), key("Escape"), check("dialog", None),
                       click(100, 478), check("dialog", "Compose"),
                       check("fields.subject", "Delivery needs review"),
                       check("editor", "A saved message for the outgoing recovery flow.", "contains"),
                       check("draft_count", 1), shot("reviewed-delivery-returned-draft"))

    def test_outbox_compact_dark(self):
        self.mcp.call("desktop.start", width=900, height=640, outgoing_mail=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(563, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), click(91, 478), check("dialog", "Outbox"),
                       check("outgoing_rows.0.delivery", "Uncertain"), shot("outbox-delivery-review-dark-compact"),
                       click(260, 462), check("outgoing_pending", 2), check("outgoing_confirmed", False),
                       click(212, 423), check("outgoing_confirmed", True), click(260, 462),
                       check("outgoing_rows.0.delivery", "Accepted"), check("outgoing_confirmed", False),
                       check("draft_count", 0), check("busy", []), wait(150), shot("outbox-recorded-sent-compact"),
                       click(529, 390), check("outgoing_pending", 1),
                       check("outgoing_rows.0.subject", "Sent copy needs review"),
                       check("busy", []), wait(150), shot("outbox-copy-recovery-compact"),
                       click(529, 474), check("outgoing_pending", 0), check("outgoing_rows", []),
                       shot("outbox-empty-compact"), key("Escape"), check("dialog", None),
                       click(87, 359), check("folder", "Sent"), check("total", 2), shot("local-sent-copies-compact"))

    def test_google_partial_permissions(self):
        for mode in ("drive", "calendar", "read-only"):
            self.mcp.call("desktop.start", google_permissions=mode)
            self.mcp.batch(check("google_connected", True), check("google_grant.access.known", True),
                           check("google_grant.access.drive", mode == "drive"),
                           check("google_grant.access.calendar_read", mode != "drive"),
                           key("ctrl+comma"), check("tab", "Preferences"),
                           click(470, 156), check("settings_tab", "Calendars"),
                           click(1240, 820), {"type": "scroll", "amount": 8}, wait(150), shot("google-permissions-" + mode),
                           key("ctrl+2"), check("tab", "Calendar"), click(1260, 348), check("dialog", "Event"),
                           check("event_access.update", mode == "calendar"), shot("google-event-" + mode),
                           key("Escape"), check("dialog", None), key("ctrl+comma"),
                           click(559, 156), check("settings_tab", "Backups"), shot("google-backup-" + mode),
                           key("ctrl+1"), check("tab", "Mail"), key("Down"),
                           check("selected", "Your weekly workspace digest"))

    def test_google_readonly_permissions_compact_dark(self):
        self.mcp.call("desktop.start", width=900, height=640, google_permissions="read-only")
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(563, 366), check("dark", True),
                       click(470, 156), check("settings_tab", "Calendars"),
                       click(764, 549), {"type": "scroll", "amount": 12}, wait(150),
                       check("google_grant.access.calendar_read", True), check("google_grant.access.calendar_write", False),
                       shot("google-permissions-compact-dark"), key("ctrl+1"), check("tab", "Mail"),
                       key("Down"), check("selected", "Your weekly workspace digest"))

    def test_google_disconnect_preserves_cached_calendars(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"),
                       click(470, 156), check("settings_tab", "Calendars"),
                       check("google_lifecycle.disconnected", False), check("events", 5), shot("google-connection-light"),
                       click(502, 850), check("dialog", "GoogleDisconnect"), shot("google-disconnect-review-light"),
                       click(664, 553), check("dialog", None), check("google_lifecycle.disconnected", False),
                       click(502, 850), check("dialog", "GoogleDisconnect"), key("Escape"), check("dialog", None),
                       click(502, 850), check("dialog", "GoogleDisconnect"), click(545, 553),
                       check("dialog", None), check("google_lifecycle.disconnected", True),
                       check("google_lifecycle.cleanup_pending", False), check("google_connected", False),
                       check("google_archived", "preview-calendar", "contains"), check("calendar_count", 2),
                       check("account_count", 2), check("events", 5), shot("google-disconnected-light"),
                       key("ctrl+2"), check("tab", "Calendar"), shot("google-calendar-offline"),
                       click(1260, 348), check("dialog", "Event"), check("event_access.update", False),
                       check("event_access.delete", False), shot("google-offline-event-read-only"),
                       key("Escape"), check("dialog", None), key("ctrl+1"), check("tab", "Mail"),
                       check("total", 120), key("Down"), check("selected", "Your weekly workspace digest"))

    def test_google_disconnect_compact_dark(self):
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(563, 366), check("dark", True),
                       click(470, 156), check("settings_tab", "Calendars"), shot("google-connection-compact-dark"))
        self.mcp.batch(click(764, 549), {"type": "scroll", "amount": 9}, wait(150), shot("google-connection-controls-compact"),
                       click(480, 482), check("dialog", "GoogleDisconnect"), shot("google-disconnect-review-compact"),
                       click(269, 412), check("dialog", None), check("google_lifecycle.disconnected", True),
                       check("google_lifecycle.cleanup_pending", False), check("events", 5),
                       shot("google-disconnected-compact"), key("ctrl+1"), check("tab", "Mail"),
                       key("Down"), check("selected", "Your weekly workspace digest"), shot("mail-after-google-disconnect"))

    def test_connection_removal_review_and_cancel(self):
        self.mcp.batch(key("c"), check("dialog", "Compose"),
                       click(650, 362), type_text("A draft to review before removal"),
                       check("draft_count", 1), key("Escape"), check("dialog", None),
                       key("ctrl+comma"), check("tab", "Preferences"), click(383, 156),
                       check("settings_tab", "Accounts"), shot("accounts-removal-controls"),
                       click(1150, 334), check("dialog", "Removal"), check("removal.messages", 118),
                       check("removal.drafts", 1), shot("account-removal-light"), key("Escape"),
                       check("dialog", None), check("account_count", 2), check("draft_count", 1),
                       click(290, 156), check("settings_tab", "General"), click(690, 366), check("dark", True),
                       click(383, 156), check("settings_tab", "Accounts"), click(1150, 334),
                       check("removal.messages", 118), check("removal.drafts", 1), shot("account-removal-dark"),
                       click(890, 555), check("dialog", None), check("account_count", 1), check("draft_count", 0),
                       check("credential_cleanup", 0), shot("account-removed"),
                       key("ctrl+1"), check("tab", "Mail"), check("total", 2),
                       check("selected", "Coffee next Thursday?"), key("Down"),
                       check("selected", "Weekend plans"), shot("remaining-account-mail"))

    def test_calendar_removal_review_and_reconnect(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(445, 156),
                       check("settings_tab", "Calendars"), shot("calendar-removal-controls"),
                       click(1150, 334), check("dialog", "Removal"), check("removal.target.id", "preview-calendar"),
                       check("removal.events", 4), shot("calendar-removal-light"),
                       click(495, 550), check("dialog", None), check("calendar_count", 2), check("events", 5),
                       click(1150, 334), check("removal.events", 4), click(890, 550),
                       check("dialog", None), check("calendar_count", 1), check("events", 1),
                       check("removed_google_calendars", 1), check("credential_cleanup", 0), shot("calendar-removed"),
                       click(420, 493), check("calendar_count", 2), check("removed_google_calendars", 0),
                       check("events", 1), shot("calendar-restored"))

    def test_connection_removal_compact(self):
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(383, 156),
                       check("settings_tab", "Accounts"), click(834, 334), check("dialog", "Removal"),
                       check("removal.messages", 118), shot("account-removal-compact"),
                       key("Escape"), check("dialog", None), check("account_count", 2),
                       click(290, 156), check("settings_tab", "General"), click(563, 366), check("dark", True),
                       click(445, 156), check("settings_tab", "Calendars"), click(834, 334),
                       check("removal.events", 4), shot("calendar-removal-dark-compact"),
                       key("Escape"), check("dialog", None), check("calendar_count", 2))

    def test_account_removal_with_unfinished_move(self):
        self.mcp.call("desktop.start", pending_transfer=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(383, 156),
                       check("settings_tab", "Accounts"), click(1150, 334), check("dialog", "Removal"),
                       check("removal.transfers", 1), shot("account-removal-pending-move"),
                       click(890, 602), wait(80), check("dialog", "Removal"), check("account_count", 2),
                       click(482, 548), check("removal_cancel_transfers", True), shot("account-removal-move-confirmed"),
                       click(890, 602), check("dialog", None), check("account_count", 1),
                       key("ctrl+1"), check("tab", "Mail"), check("total", 2),
                       shot("remaining-account-after-cancelled-move"))

    def test_calendar_connection_discovery(self):
        self.mcp.call("desktop.start", empty_calendars=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(467, 156),
                       check("settings_tab", "Calendars"), shot("calendar-connections-empty"), click(365, 335), check("dialog", "Calendar"), shot("calendar-connection-form"),
                       click(690, 410), type_text("https://calendar.example.test/"), click(690, 489), type_text("alex"),
                       click(690, 569), type_text("fixture-password"), click(520, 625),
                       check("calendar_discovering", False), check("calendar_choices.0.name", "Personal plans"),
                       check("calendar_choices.1.access.create", False), check("calendar_selected", 2), shot("calendar-discovered-choices"),
                       click(482, 438), click(482, 510), check("calendar_selected", 0), click(932, 610),
                       check("dialog", "Calendar"), check("calendar_saving", False),
                       click(482, 438), click(482, 510), check("calendar_selected", 2), click(932, 610),
                       check("dialog", None), check("calendar_sources.0.name", "Personal plans"),
                       check("calendar_sources.1.name", "Team holidays"), check("calendar_sources.1.access.create", False),
                       shot("calendar-connected-choices"))
        self.mcp.batch(click(365, 474), check("dialog", "Calendar"), click(520, 625),
                       check("calendar_error", "Preview connection failed", "contains"), shot("calendar-connection-error"),
                       key("Escape"), check("dialog", None), click(293, 156), check("settings_tab", "General"),
                       click(690, 366), check("dark", True), click(467, 156), check("settings_tab", "Calendars"),
                       click(365, 474), check("dialog", "Calendar"), shot("calendar-connection-dark"),
                       click(690, 410), type_text("https://calendar.example.test/"), click(690, 489), type_text("alex"),
                       click(690, 569), type_text("fixture-password"), click(520, 625),
                       check("calendar_choices.0.name", "Personal plans"), shot("calendar-choices-dark"),
                       click(932, 610), check("dialog", None), check("calendar_saving", False))
        self.assertEqual(len(self.mcp.call("desktop.state")["calendar_sources"]), 2)

    def test_calendar_discovery_compact(self):
        self.mcp.call("desktop.start", width=900, height=640, empty_calendars=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(445, 156),
                       check("settings_tab", "Calendars"), click(343, 335), check("dialog", "Calendar"),
                       shot("calendar-connection-compact"), click(450, 270), type_text("https://calendar.example.test/"),
                       click(450, 349), type_text("alex"), click(450, 429), type_text("fixture-password"),
                       click(250, 485), check("calendar_choices.0.name", "Personal plans"), shot("calendar-choices-compact"))

    def test_calendar_read_only_event(self):
        self.mcp.call("desktop.start", readonly_calendars=True)
        self.mcp.batch(key("ctrl+2"), check("tab", "Calendar"), wait(80), click(1260, 348),
                       check("dialog", "Event"), check("fields.source", "preview-home-calendar"),
                       check("event_access.update", False), check("event_access.delete", False), shot("calendar-read-only-event"),
                       key("Escape"), check("dialog", None), double_click(700, 474), check("dialog", "Event"),
                       check("fields.source", "preview-calendar"), check("event_access.create", True), shot("calendar-writable-default"))

    def test_calendar_event_creation(self):
        self.mcp.batch(key("ctrl+2"), check("tab", "Calendar"), wait(80), double_click(700, 474),
                       check("dialog", "Event"), check("fields.all_day", "true"), shot("new-calendar-event"),
                       wait(80), type_text("A real calendar flow"), check("fields.title", "A real calendar flow"),
                       click(510, 677), check("dialog", None), check("events", 6), shot("saved-calendar-event"))

    def test_calendar_same_uid_in_different_calendars_edits_and_deletes_correct_event(self):
        self.mcp.batch(key("ctrl+2"), check("tab", "Calendar"), wait(80),
                       click(1260, 348), check("dialog", "Event"),
                       check("fields.title", "A little time outside"), check("fields.source", "preview-home-calendar"),
                       shot("calendar-duplicate-id-edit"), click(650, 313), key("ctrl+a"), type_text("More time outside"),
                       click(510, 708), check("dialog", None), check("events", 5),
                       click(1260, 348), check("dialog", "Event"), check("fields.title", "More time outside"),
                       check("fields.source", "preview-home-calendar"), click(925, 708), check("dialog", None), check("events", 4),
                       shot("calendar-scoped-deletion"), click(1260, 234), check("dialog", "Event"),
                       check("fields.title", "A quiet start"), check("fields.source", "preview-calendar"),
                       key("Escape"), check("dialog", None))

    def test_keyboard_pane_navigation_and_full_reader(self):
        self.mcp.batch(key("Down"), check("selected", "Your weekly workspace digest"),
                       key("Up"), check("selected", "A little more room to think"),
                       double_click(420, 243), check("full_reader", True), shot("full-reader"),
                       key("Escape"), check("full_reader", False),
                       key("Tab"), check("sidebar_focus", True), key("Down"), check("filter", "Flagged"),
                       key("Tab"), check("sidebar_focus", False), click(80, 278), check("filter", "All"),
                       check("selected", "A little more room to think"), click(420, 244), check("sidebar_focus", False))
        for _ in range(12): self.mcp.batch(key("Down"), wait(25))
        self.mcp.batch(check("inbox_scroll", 200, "gte"), shot("keyboard-scrolled-inbox"))

    def test_sidebar_combined_folders_and_account_collapse(self):
        self.mcp.call("desktop.start", long_folders=True)
        long_folder = "Mailspring/Snoozed/Worldwide correspondence and scheduled delivery"
        self.mcp.batch(shot("sidebar-long-labels-light"), click(100, 537), check("folder", "Projects"), check("total", 1),
                       {"type": "click", "x": 100, "y": 576, "modifiers": ["ctrl"]}, check("total", 2),
                       check("selected_folders.1.folder", long_folder), shot("sidebar-combined-folders"),
                       {"type": "click", "x": 100, "y": 537, "modifiers": ["ctrl"]}, check("total", 1),
                       check("selected_folders.0.folder", long_folder),
                       click(100, 615), check("folder", "WWW MMM WWW MMM WWW MMM WWW MMM"), check("selected_folders", None),
                       {"type": "click", "x": 100, "y": 615, "modifiers": ["ctrl"]}, check("selected_folders", []), check("total", 0),
                       click(100, 498), check("collapsed_accounts", ["preview-work"]), check("preferences_saved", True), shot("sidebar-account-collapsed"),
                       click(100, 498), check("collapsed_accounts", []), check("preferences_saved", True),
                       {"type": "hover", "x": 125, "y": 576}, wait(600), shot("sidebar-long-name-truncation"),
                       click(78, 278), check("folder", "INBOX"), check("selected_folders", None), check("total", 120))

    def test_sidebar_long_labels_compact_dark(self):
        self.mcp.call("desktop.start", width=900, height=640, long_folders=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(563, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), shot("sidebar-compact-dark"),
                       {"type": "hover", "x": 110, "y": 520}, {"type": "scroll", "amount": 4}, wait(120),
                       shot("sidebar-compact-dark-folders"), {"type": "scroll", "amount": -20}, wait(100),
                       click(155, 278), check("inbox_expanded", True), shot("sidebar-compact-expanded"))

    def test_mouse_flagging_and_unified_expansion(self):
        self.mcp.batch(click(570, 215), check("starred", False), click(570, 215), check("starred", True), shot("flagged-message-red-outline"),
                       click(186, 278), check("inbox_expanded", True), shot("expanded-unified-inbox"),
                       click(104, 357), check("account", "preview-personal"), check("total", 2),
                       click(186, 278), check("inbox_expanded", False), shot("account-inbox"))

    def test_empty_calendar_offers_connection(self):
        self.mcp.call("desktop.start", empty_calendars=True)
        self.mcp.batch(key("ctrl+2"), check("tab", "Calendar"), wait(80), double_click(700, 474),
                       check("dialog", "Event"), check("calendar_connected", False), shot("empty-calendar-event"))

    def test_reply_history_sender_attachments_and_image_exceptions(self):
        for index, x in enumerate([701, 800, 910]):
            if index: self.mcp.call("desktop.start")
            self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("prottoype"), check("total", 1), key("Escape"),
                           check("reply_count", 1), check("attachment_count", 4), check("images_allowed", False),
                           click(740, 475), check("expanded_replies", 0, "contains"), shot("expanded-reply"),
                           click(740, 475), check("expanded_replies", []),
                           click(740, 223), check("dialog", "Sender"), shot("sender-details"), key("Escape"), check("dialog", None),
                           click(x, 360), check("images_allowed", True), shot(f"image-exception-{index}"))
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(730, 156), check("settings_tab", "Privacy"), shot("privacy-preferences"))

    def test_reading_preferences_and_cross_account_move(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), wait(80),
                       click(286, 737), check("unified", False), click(286, 737), check("unified", True),
                       click(286, 773), check("cross_account_moves", True),
                       click(1145, 566), wait(80), shot("font-size-menu"), click(1140, 173), check("reader_size", 11),
                       key("ctrl+1"), check("tab", "Mail"), wait(80), key("m"), check("dialog", "Move"), shot("cross-account-destination"),
                       click(710, 327), wait(80), click(710, 403), check("fields.move_account", "preview-personal"),
                       click(670, 385), type_text("archvie"), key("Return"), check("dialog", None), check("total", 119),
                       click(183, 278), check("inbox_expanded", True), click(104, 357), check("account", "preview-personal"), click(183, 278), check("inbox_expanded", False), click(85, 399), check("folder", "Archive"),
                       check("total", 1), check("selected", "A little more room to think"), shot("transferred-message"))

    def test_account_wizard_security_and_connection_testing(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(383, 156), check("settings_tab", "Accounts"), shot("account-settings-before-add"), click(354, 474), check("dialog", "Account"), shot("account-identity"),
                       click(674, 444), type_text("Fastmail"), check("fields.name", "Fastmail"), click(664, 524), type_text("test@example.com"), check("fields.email", "test@example.com"),
                       click(572, 580), check("fields.host", "imap.fastmail.com"), click(936, 635), check("fields.setup_step", "1"), shot("account-incoming"),
                       click(690, 408), wait(80), click(690, 482), check("fields.incoming_security", "StartTls"), check("fields.port", "143"),
                       click(560, 701), check("fields.test_incoming", "Test workspaces do not connect", "contains"), shot("tested-incoming"), click(683, 179), check("fields.setup_step", "2"), shot("account-smtp"), click(686, 347), wait(80), click(686, 420),
                       check("fields.smtp_security", "StartTls"), check("fields.smtp_port", "587"),
                       click(544, 598), check("fields.test_smtp", "Test workspaces do not connect", "contains"), shot("tested-smtp"),
                       click(700, 772), type_text("Sent Mail"), check("fields.sent_folder", "Sent Mail"),
                       click(704, 692), wait(100), shot("sent-copy-policy-menu"),
                       click(664, 620), check("fields.sent_copy", "ServerManaged"),
                       click(704, 692), wait(100), click(664, 655), check("fields.sent_copy", "LocalOnly"),
                       check("fields.sent_folder", "Sent Mail"), shot("local-sent-policy"),
                       click(923, 788), check("notice", "Account changes are disabled in preview", "contains"),
                       check("dialog", "Account"), check("fields.sent_copy", "LocalOnly"),
                       check("fields.sent_folder", "Sent Mail"), check("account_count", 2))

    def test_background_sync_keeps_navigation_responsive(self):
        for appearance in ("light", "dark"):
            if appearance == "dark":
                self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"),
                               click(690, 366), check("dark", True),
                               key("ctrl+1"), check("tab", "Mail"))
            self.mcp.batch(shot(f"compact-header-{appearance}"),
                           click(1400, 36), check("busy", "sync", "contains"),
                           shot(f"compact-header-syncing-{appearance}"),
                           click(87, 159), check("tab", "Calendar"),
                           shot(f"responsive-during-sync-{appearance}"), check("busy", []))

    def test_backup_preferences_and_setup(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"),
                       click(559, 156), check("settings_tab", "Backups"), shot("backup-setup"),
                       click(525, 744), check("notice", "at least 12 characters", "contains"),
                       click(520, 642), type_text("a fixture backup passphrase"),
                       click(500, 414), type_text("relative-folder"), click(525, 744),
                       check("notice", "absolute backup folder", "contains"),
                       click(500, 414), key("ctrl+a"), type_text("/tmp/shep-e2e-backup-preview"),
                       click(400, 494), key("ctrl+a"), type_text("12"), click(525, 744),
                       check("notice", "Backup is disabled in preview.", "contains"),
                       check("preferences_saved", True), check("saved_backup_folder", "/tmp/shep-e2e-backup-preview"),
                       check("saved_backup_copies", 12), shot("backup-settings-saved-before-action"),
                       click(288, 539), check("auto_backup", True), click(1340, 87),
                       check("preferences_saved", True), check("saved_auto_backup", True), check("backup_ready", False),
                       shot("automatic-backup-needs-first-copy"),
                       click(290, 156), check("settings_tab", "General"), click(690, 366), check("dark", True),
                       click(559, 156), check("settings_tab", "Backups"), shot("backup-setup-dark"))

    def test_native_navigation_performance_gate(self):
        timings = []
        for index in range(30):
            started = time.monotonic()
            expected = "Calendar" if index % 2 == 0 else "Mail"
            self.mcp.batch(key("ctrl+2" if index % 2 == 0 else "ctrl+1"), check("tab", expected))
            timings.append((time.monotonic() - started) * 1000)
        timings.sort()
        p95 = timings[math.ceil(len(timings) * .95) - 1]
        state = self.mcp.call("desktop.state")
        report = {"samples": len(timings), "metrics_ms": {
            "ui_handler_p95": state["update_p95_ms"], "native_navigation_p95": p95}}
        directory = ROOT / "artifacts" / "performance"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "ui.json").write_text(json.dumps(report, indent=2))
        self.assertLess(p95, 150, f"Native navigation p95 was {p95:.2f} ms")
        self.assertLess(state["update_p95_ms"], 8)

    def test_resize_divider_with_mouse_and_persist(self):
        self.mcp.batch(drag(616, 500, 785, 500), check("reader_split", .44, "gte"),
                       check("saved_reader_split", .44, "gte"), shot("resized-inbox"),
                       key("ctrl+2"), check("tab", "Calendar"), key("ctrl+1"), check("tab", "Mail"),
                       check("reader_split", .44, "gte"),
                       wait(400), drag(785, 500, 450, 500), check("reader_split", .3, "lte"), shot("narrow-inbox"))

    def test_flag_filter_sort_and_paging(self):
        self.mcp.batch(key("s"), check("starred", False), key("s"), check("starred", True), shot("flagged-message"),
                       click(84, 320), check("filter", "Flagged"), check("total", 1, "gte"),
                       click(80, 278), check("folder", "INBOX"),
                       click(350, 100), wait(250), shot("filter-menu"), click(350, 155), check("filter", "Unread"),
                       shot("unread-filter"),
                       click(350, 100), wait(250), click(350, 125), check("filter", "All"),
                       click(531, 100), wait(250), shot("sort-menu"), click(531, 155), check("sort", "Oldest"),
                       shot("oldest-first"),
                       click(583, 884), check("offset", 50), shot("next-page"))

    def test_compact_window_layout(self):
        self.mcp.call("desktop.start", width=900, height=640)
        self.mcp.batch(check("ready", True), shot("mail-compact"),
                       click(852, 34), check("busy", "sync", "contains"), shot("mail-compact-syncing"),
                       click(400, 245), check("selected", "A little more room to think"),
                       key("Down"), check("selected", "Your weekly workspace digest"),
                       key("ctrl+2"), check("tab", "Calendar"), shot("calendar-compact"),
                       key("ctrl+comma"), check("tab", "Preferences"), shot("preferences-compact"),
                       click(559, 156), check("settings_tab", "Backups"), shot("backups-compact"))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture-only", action="store_true")
    parser.add_argument("--functional-only", action="store_true", help="Defer performance measurements on a busy machine")
    args, rest = parser.parse_known_args()
    if args.capture_only:
        client = McpClient()
        try:
            print(json.dumps(client.call("desktop.start"), indent=2))
            print(json.dumps(client.batch(wait(400), shot("initial")), indent=2))
        finally:
            client.close()
    elif args.functional_only:
        names = [name for name in unittest.defaultTestLoader.getTestCaseNames(NativeFlows) if "performance" not in name]
        suite = unittest.TestSuite(NativeFlows(name) for name in names)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        sys.exit(0 if result.wasSuccessful() else 1)
    else:
        unittest.main(argv=[sys.argv[0], *rest], verbosity=2)


if __name__ == "__main__":
    main()
