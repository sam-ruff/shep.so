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
import sqlite3

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
def paste_text(value): return {"type": "paste", "text": value}
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

    def hold_mail_over(self, source_x, source_y, target_x, target_y):
        self.mcp.batch({"type":"hover","x":source_x,"y":source_y},{"type":"mouse_down"},
                       {"type":"hover","x":target_x,"y":target_y},check("mail_drag.active",True))

    def selected_mail_subject(self):
        # Action identity is ready with metadata. The reader's `selected`
        # subject can still be None while its body loads independently.
        state = self.mcp.call("desktop.state")
        mail = next((mail for mail in state["mail_rows"] if mail["id"] == state["selected_id"]), None)
        self.assertIsNotNone(mail, "The selected action target must exist in the metadata page")
        return mail["subject"]

    def test_nested_folder_roots_mouse_selection_and_restart(self):
        result=self.mcp.call("desktop.start",nested_folders=True,persistent=True)
        print(f"Nested folder persistence evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(check("expanded_folders",{}),check("sidebar_labels","Projects","contains"),
                       check("sidebar_labels","Teams","contains"),check("sidebar_labels","Notes/flat.name","contains"),
                       shot("nested-folder-roots"),click(85,540),check("folder","Projects"),
                       check("selected","Project overview"),check("expanded_folders",{}),
                       click(188,540),check("expanded_folders.preview-work","Projects","contains"),
                       check("sidebar_labels","Design","contains"),check("folder","Projects"),
                       shot("nested-projects-expanded"),click(176,584),
                       check("expanded_folders.preview-work","Projects/Design","contains"),
                       check("sidebar_labels","日本語","contains"),click(110,626),
                       check("folder","Projects/Design/&ZeVnLIqe-"),check("selected","Japanese folder note"),
                       shot("nested-japanese-folder-open"),click(176,540),
                       check("expanded_folders.preview-work",["Projects/Design"]),
                       check("folder","Projects/Design/&ZeVnLIqe-"),check("preferences_saved",True),
                       check("saved_expanded_folders.preview-work",["Projects/Design"]),shot("nested-parent-collapsed"),
                       {"type":"restart"},check("expanded_folders.preview-work",["Projects/Design"]),
                       click(188,540),check("sidebar_labels","日本語","contains"),
                       shot("nested-expansion-restored"))

    def test_nested_folder_keyboard_containers_and_literal_delimiters(self):
        result=self.mcp.call("desktop.start",nested_folders=True)
        print(f"Nested folder keyboard evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(click(85,584),check("expanded_folders.preview-work","Teams","contains"),
                       check("folder","INBOX"),check("sidebar_index",7),key("Right"),check("sidebar_index",8),
                       key("Return"),check("expanded_folders.preview-work","Teams/Remote","contains"),
                       check("folder","INBOX"),key("Right"),check("sidebar_index",9),key("Return"),
                       check("folder","Teams/Remote/Meetings"),check("selected","Remote team agenda"),
                       shot("nested-container-keyboard-open"),key("Left"),check("sidebar_index",8),key("Left"),
                       check("expanded_folders.preview-work",["Teams"]),key("Left"),check("sidebar_index",7),key("Left"),
                       check("expanded_folders.preview-work",[]),click(85,727),
                       check("expanded_folders.preview-personal",["Home"]),key("Right"),check("sidebar_index",11),
                       key("Return"),check("folder","Home.Plans"),check("selected","Home plans"),
                       key("Right"),check("sidebar_labels","2026","contains"),key("Right"),key("Return"),
                       check("folder","Home.Plans.2026"),check("selected","Plans for 2026"),shot("nested-dot-delimiter"),
                       key("Left"),key("Left"),key("Left"),key("Left"),
                       check("expanded_folders.preview-personal",[]),click(85,769),
                       check("folder","Notes/flat.name"),check("selected","A flat folder"),
                       click(85,626),check("folder","Notes/flat.name"),shot("nested-literal-flat-and-disabled-container"))

    def test_nested_folder_drag_reveals_containers_and_undo(self):
        result=self.mcp.call("desktop.start",nested_folders=True,mail_actions="slow")
        print(f"Nested folder drag evidence: {result['artifacts']}",flush=True)
        self.hold_mail_over(402,347,85,584)
        self.mcp.batch(check("expanded_folders.preview-work","Teams","contains"),
                       check("mail_drag.target",None),shot("nested-drag-container-expanded"),
                       {"type":"hover","x":95,"y":626},
                       check("expanded_folders.preview-work","Teams/Remote","contains"),
                       check("mail_drag.target",None),{"type":"hover","x":110,"y":668},
                       check("mail_drag.target","Teams/Remote/Meetings"),check("mail_drag.valid",True),
                       shot("nested-drag-leaf-target"),{"type":"mouse_up"},check("total",119),check("mail_pending",1),
                       check("action_toast.label","Moved 1 message to Teams/Remote/Meetings"),
                       click(1340,874),check("total",120),{**check("mail_pending",0),"timeout_ms":5000},
                       check("mail_rows.1.subject","Your weekly workspace digest"),shot("nested-drag-restored"))

    def test_nested_folder_unicode_move_review_and_combined_selection(self):
        result=self.mcp.call("desktop.start",nested_folders=True)
        print(f"Nested folder Move evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(click(402,347),key("m"),check("dialog","Move"),check("focused_input","folder-search"),paste_text("日本語"),
                       check("move_enter_destination","Projects/Design/&ZeVnLIqe-"),shot("nested-unicode-move-search"),
                       key("Return"),check("total",119),check("mail_pending",0),
                       check("action_toast.label","Moved 1 message to Projects/Design/日本語"),shot("nested-unicode-move-toast"),
                       click(1340,874),check("total",120),check("mail_pending",0),
                       click(584,164),check("mail_selection.mode",True),check("mail_selection.drawn",True),
                       click(274,218),click(274,322),check("mail_selection.count",2),check("mail_selection.pending",False),
                       key("m"),check("dialog","Move"),check("focused_input","folder-search"),paste_text("日本語"),
                       check("move_enter_destination","Projects/Design/&ZeVnLIqe-"),key("Return"),
                       check("dialog","BulkReview"),check("bulk.action","Move to Projects/Design/日本語"),
                       shot("nested-unicode-bulk-review"),key("n"),check("dialog",None),check("total",120),
                       click(85,540),check("folder","Projects"),click(188,540),
                       check("sidebar_labels","Design","contains"),
                       {**click(95,584),"modifiers":["ctrl"]},check("total",2),
                       check("selected_folders",[{"account":"preview-work","folder":"Projects","sent_only":False},
                                                 {"account":"preview-work","folder":"Projects/Design","sent_only":False}]),
                       {**click(95,668),"modifiers":["ctrl"]},check("expanded_folders.preview-work","Teams","contains"),
                       check("total",2),shot("nested-parent-child-combined-inbox"))

    def test_nested_folder_compact_dark_keyboard_reveal_and_saved_size(self):
        result=self.mcp.call("desktop.start",nested_folders=True,persistent=True)
        print(f"Compact folder tree evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(80),click(690,366),check("dark",True),
                       key("ctrl+1"),check("tab","Mail"),click(188,540),check("sidebar_labels","Design","contains"),
                       click(176,584),check("sidebar_labels","日本語","contains"),click(110,626),
                       check("selected","Japanese folder note"),{"type":"resize","width":900,"height":640},
                       check("window_size",[900,640]),key("Left"),check("sidebar_index",7),
                       key("Right"),check("sidebar_index",8),key("Return"),check("selected","Japanese folder note"),
                       shot("nested-compact-dark-keyboard-revealed"),
                       click(100,529),check("folder","Projects/Design/&ZeVnLIqe-"),
                       check("saved_window_size",{"width":900.,"height":640.}),check("preferences_saved",True),
                       {"type":"restart"},check("dark",True),check("window_size",[900,640]),
                       check("sidebar_labels","日本語","contains"),shot("nested-compact-dark-restarted"))
        self.mcp.batch({"type":"resize","width":1440,"height":920},key("ctrl+comma"),
                       check("tab","Preferences"),wait(120),click(1145,623),wait(80),click(1140,509),
                       check("interface_scale",120),check("preferences_saved",True),key("ctrl+1"),
                       check("tab","Mail"),wait(120),click(105,648),check("folder","Projects"),
                       key("Right"),check("sidebar_index",7),key("Right"),check("sidebar_index",8),
                       key("Return"),check("selected","Japanese folder note"),shot("nested-dark-large-scale"))

    def test_drag_single_message_uses_the_source_row_and_immediate_undo(self):
        started=self.mcp.call("desktop.start",mail_actions="slow")
        print(f"Single mail drag evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(check("selected","A little more room to think"))
        self.hold_mail_over(402,347,85,399)
        self.mcp.batch(check("mail_drag.count",1),check("mail_drag.target","Archive"),check("mail_drag.valid",True),
                       shot("drag-single-archive-hover"),{"type":"mouse_up"},check("total",119),
                       check("selected","A little more room to think"),check("mail_pending",1),
                       check("action_toast.label","Archived 1 message"),shot("drag-archive-saving"),
                       click(1340,874),check("total",120),{**check("mail_pending",0),"timeout_ms":5000},
                       check("mail_rows.1.subject","Your weekly workspace digest"),shot("drag-archive-restored"))

    def test_drag_group_review_cancel_and_mixed_account_trash(self):
        self.mcp.batch(click(584,164),check("mail_selection.mode",True),check("mail_selection.drawn",True),
                       click(274,218),click(274,426),check("mail_selection.count",2),check("mail_selection.pending",False))
        self.hold_mail_over(402,245,85,399)
        self.mcp.batch(check("mail_drag.count",2),check("mail_drag.valid",True),shot("drag-group-archive-hover"),
                       {"type":"mouse_up"},check("dialog","BulkReview"),check("bulk.review_count",2),
                       check("total",120),shot("drag-group-review"),key("n"),check("dialog",None),
                       check("mail_selection.count",2),check("mail_rows.0.unread",True),check("mail_rows.2.unread",True))
        self.hold_mail_over(402,245,85,438)
        self.mcp.batch(check("mail_drag.target","Trash"),{"type":"mouse_up"},check("dialog","BulkReview"),
                       check("bulk.review_count",2),shot("drag-group-trash-review"),key("y"),
                       check("total",118),check("bulk.jobs.0.completed",2),shot("drag-group-trash-complete"))

    def test_drag_escape_outside_and_same_folder_preserve_selection(self):
        self.mcp.batch(click(584,164),check("mail_selection.mode",True),check("mail_selection.drawn",True),
                       click(274,218),click(274,322),check("mail_selection.count",2),check("mail_selection.pending",False))
        self.hold_mail_over(402,245,85,399)
        self.mcp.batch(key("Escape"),check("mail_drag.active",False),check("mail_selection.count",2),
                       {"type":"mouse_up"},check("total",120),check("dialog",None))
        self.hold_mail_over(402,245,402,255)
        self.mcp.batch({"type":"click","x":402,"y":255,"button":3},check("mail_drag.active",False),
                       check("context_menu",None),{"type":"mouse_up"},check("mail_selection.count",2))
        self.hold_mail_over(402,245,800,500)
        self.mcp.batch(check("mail_drag.target",None),{"type":"mouse_up"},check("mail_selection.count",2),check("total",120))
        self.hold_mail_over(402,245,85,278)
        self.mcp.batch(check("mail_drag.target","INBOX"),check("mail_drag.valid",False),
                       check("mail_drag.reason","already","contains"),shot("drag-already-in-inbox"),
                       {"type":"mouse_up"},check("notice","already","contains"),check("dialog",None),
                       check("total",120),check("mail_selection.count",2),check("mail_pending",0))

    def test_drag_cross_account_preference_rejection_and_enabled_transfer(self):
        self.hold_mail_over(402,245,95,636)
        self.mcp.batch(check("mail_drag.account","preview-personal"),check("mail_drag.valid",False),
                       check("mail_drag.reason","Preferences","contains"),shot("drag-cross-account-disabled"),
                       {"type":"mouse_up"},check("total",120),check("mail_pending",0),check("notice","Preferences","contains"),
                       key("ctrl+comma"),check("tab","Preferences"),wait(80),click(286,773),
                       check("cross_account_moves",True),key("ctrl+1"),check("tab","Mail"),wait(80))
        self.hold_mail_over(402,245,95,636)
        self.mcp.batch(check("mail_drag.valid",True),shot("drag-cross-account-enabled"),{"type":"mouse_up"},
                       check("total",119),check("mail_pending",0),click(95,636),check("folder","Projects"),
                       check("selected","A little more room to think"),check("mail_rows.0.account_id","preview-personal"),
                       shot("drag-cross-account-destination"))

    def test_drag_hover_expands_accounts_and_unified_inbox(self):
        self.mcp.batch(click(85,497),check("collapsed_accounts","preview-work","contains"),wait(80))
        self.hold_mail_over(402,245,85,497)
        self.mcp.batch(check("collapsed_accounts",[]),shot("drag-account-expanded"),
                       {"type":"hover","x":85,"y":536},check("mail_drag.target","Projects"),
                       check("mail_drag.account","preview-work"),check("mail_drag.valid",True),
                       {"type":"mouse_up"},check("total",119),check("mail_pending",0),
                       click(85,536),check("folder","Projects"),check("selected","A little more room to think"))
        self.hold_mail_over(402,245,85,278)
        self.mcp.batch(check("inbox_expanded",True),check("mail_drag.target","INBOX"),
                       check("mail_drag.valid",True),shot("drag-unified-expanded"),{"type":"mouse_up"},
                       check("total",0),check("mail_pending",0),click(85,278),check("folder","INBOX"),check("total",120))

    def test_drag_failure_rolls_back_and_keeps_other_navigation_usable(self):
        self.mcp.call("desktop.start",mail_actions="fail")
        self.hold_mail_over(402,347,85,399)
        self.mcp.batch({"type":"mouse_up"},check("total",119),check("mail_pending",1),
                       check("action_toast.label","Archived 1 message"),
                       key("ctrl+comma"),check("tab","Preferences"),shot("drag-failure-preferences-usable"),
                       {**check("mail_pending",0),"timeout_ms":5000},check("notice","restored","contains"),
                       key("ctrl+1"),check("tab","Mail"),check("total",120),
                       check("mail_rows.1.subject","Your weekly workspace digest"),shot("drag-failure-restored"))

    def test_drag_pop3_rejects_cross_account_but_allows_local_folder_moves(self):
        self.mcp.call("desktop.start",pop3_account=True)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(80),click(286,773),
                       check("cross_account_moves",True),key("ctrl+1"),check("tab","Mail"),wait(80))
        self.hold_mail_over(402,245,95,636)
        self.mcp.batch(check("mail_drag.valid",False),check("mail_drag.reason","IMAP","contains"),
                       shot("drag-pop3-cross-account-rejected"),{"type":"mouse_up"},check("total",120),check("mail_pending",0))
        self.hold_mail_over(402,450,95,636)
        self.mcp.batch(check("mail_drag.valid",True),shot("drag-pop3-local-folder"),{"type":"mouse_up"},
                       check("total",119),check("mail_pending",0),click(95,636),check("folder","Projects"),
                       check("selected","Coffee next Thursday?"),shot("drag-pop3-local-moved"))

    def test_drag_compact_dark_and_large_interface_scale(self):
        self.mcp.call("desktop.start",mail_actions="slow")
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(80),click(690,366),check("dark",True),
                       key("ctrl+1"),check("tab","Mail"),{"type":"resize","width":900,"height":640},wait(120))
        self.hold_mail_over(370,245,85,399)
        self.mcp.batch(check("mail_drag.valid",True),shot("drag-compact-dark-hover"),{"type":"mouse_up"},
                       check("total",119),check("mail_pending",1),click(800,594),check("total",120),
                       {**check("mail_pending",0),"timeout_ms":5000},{"type":"resize","width":1440,"height":920},
                       key("ctrl+comma"),check("tab","Preferences"),wait(120),click(1145,623),wait(80),click(1140,509),
                       check("interface_scale",120),check("preferences_saved",True),key("ctrl+1"),check("tab","Mail"),wait(120))
        self.hold_mail_over(480,295,102,478)
        self.mcp.batch(check("mail_drag.target","Archive"),check("mail_drag.valid",True),shot("drag-large-scale-hover"),
                       {"type":"mouse_up"},check("total",119),{**check("mail_pending",0),"timeout_ms":5000},shot("drag-large-scale-moved"))

    def test_drag_can_scroll_to_a_folder_while_holding_the_message(self):
        result = self.mcp.call("desktop.start",width=900,height=640,long_folders=True)
        directory = Path(result["artifacts"])
        print(f"Scrolled drag rendering evidence: {directory}", flush=True)
        self.hold_mail_over(370,245,110,520)
        self.mcp.batch({"type":"scroll","amount":4},wait(120),shot("drag-scrolled-folder-list"),
                       {"type":"hover","x":85,"y":438},check("mail_drag.target","家族のカレンダーと旅行の計画と写真"),
                       check("mail_drag.valid",True),shot("drag-unicode-folder-hover"))
        # A previous label sat above the Preferences footer. Its shadow must be
        # erased when the pointer moves, without needing a full-window repaint.
        pixels = subprocess.check_output(["convert",str(directory / "drag-unicode-folder-hover.webp"),
                                          "-crop","48x8+130+587","+repage","-colorspace","Gray","-depth","8","gray:-"])
        self.assertLessEqual(max(pixels)-min(pixels),16,"Moving the drag label left a shadow trail")
        self.mcp.batch({"type":"mouse_up"},
                       check("total",119),check("mail_pending",0),click(85,438),
                       check("folder","家族のカレンダーと旅行の計画と写真"),check("total",2),shot("drag-unicode-folder-moved"))

    def test_drag_selection_across_pages_moves_the_entire_reviewed_group(self):
        self.mcp.batch(click(402,245),key("ctrl+a"),check("mail_selection.count",120),
                       check("mail_selection.pending",False),click(583,884),check("offset",50),
                       check("mail_selection.pending",False),wait(80))
        self.hold_mail_over(402,245,85,399)
        self.mcp.batch(check("mail_drag.count",120),check("mail_drag.valid",True),shot("drag-all-pages-hover"),
                       {"type":"mouse_up"},check("dialog","BulkReview"),check("bulk.review_count",120),
                       shot("drag-all-pages-review"),key("Return"),check("total",0),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},check("bulk.jobs.0.completed",120),
                       click(85,399),check("folder","Archive"),check("total",120),shot("drag-all-pages-archived"))

    def archive_two_for_recovery(self):
        self.mcp.batch(click(584,164),check("mail_selection.mode",True),check("mail_selection.drawn",True),
                       click(274,218),click(274,322),check("mail_selection.count",2),
                       check("mail_selection.pending",False),key("Delete"),check("dialog","BulkReview"),
                       key("Return"),check("dialog",None),check("bulk.jobs.0.running",1))

    def test_bulk_graceful_close_preserves_current_receipt_and_resumes_queued_mail(self):
        started=self.mcp.call("desktop.start",persistent=True,mail_actions="slow")
        print(f"Graceful group restart evidence: {started['artifacts']}",flush=True)
        self.archive_two_for_recovery()
        closed=self.mcp.call("desktop.close")
        self.assertEqual(closed["returncode"],0)
        # Read-only inspection while the owned app is closed proves its close
        # handler saved one receipt and left the other queued for the next process.
        database=Path(started["artifacts"])/"fixture.sqlite"
        with sqlite3.connect(database.as_uri()+"?mode=ro",uri=True) as cache:
            self.assertEqual(cache.execute("SELECT status FROM bulk_items ORDER BY position").fetchall(),[("done",),("queued",)])
            self.assertEqual(cache.execute("SELECT count(*) FROM messages WHERE folder='Archive'").fetchone()[0],1)
        restarted=self.mcp.call("desktop.restart")
        self.assertNotEqual(started["pid"],restarted["pid"])
        self.mcp.batch({**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.completed",2),check("bulk.jobs.0.uncertain",0),check("total",118),
                       click(1330,36),check("dialog","BulkHistory"),wait(80),click(700,490),
                       check("bulk.items.1.status","done"),shot("bulk-graceful-restart-receipts"))

    def test_bulk_crash_restart_keeps_unconfirmed_results_for_review(self):
        started=self.mcp.call("desktop.start",persistent=True,mail_actions="slow")
        print(f"Crash group restart evidence: {started['artifacts']}",flush=True)
        self.archive_two_for_recovery()
        self.mcp.batch({"type":"restart","crash":True},
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.uncertain",1),check("bulk.jobs.0.completed",1),check("total",119),
                       click(1330,36),check("dialog","BulkHistory"),wait(80),click(700,490),
                       check("bulk.items.0.status","uncertain"),check("bulk.items.1.status","done"),
                       shot("bulk-crash-unconfirmed-review"),click(550,480),check("bulk.resolving",None,"ne"),
                       shot("bulk-resolution-confirmation"),key("n"),check("bulk.resolving",None),
                       check("bulk.jobs.0.uncertain",1),wait(80),click(550,480),check("bulk.resolving",None,"ne"),
                       key("Escape"),check("bulk.resolving",None),check("dialog","BulkHistory"),
                       wait(80),click(550,480),check("bulk.resolving",None,"ne"),key("y"),
                       check("bulk.jobs.0.uncertain",0),check("bulk.jobs.0.cancelled",1),check("total",119),
                       check("bulk.items.0.status","cancelled"),shot("bulk-accepted-current-state"),
                       {"type":"restart"},check("bulk.jobs.0.cancelled",1),check("bulk.jobs.0.uncertain",0),
                       click(1330,36),check("dialog","BulkHistory"),wait(80),click(700,490),
                       check("bulk.items.1.status","done"),wait(80),click(560,440),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},check("bulk.jobs.0.restored",1),
                       check("bulk.jobs.0.cancelled",1),check("total",120),shot("bulk-review-retains-other-undo"))

    def test_bulk_history_retry_undo_after_restart(self):
        started=self.mcp.call("desktop.start",persistent=True,mail_actions="slow",undo_failure_once=True)
        print(f"History Undo restart evidence: {started['artifacts']}",flush=True)
        self.archive_two_for_recovery()
        self.mcp.batch({**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},check("bulk.jobs.0.completed",2),
                       click(1330,36),check("dialog","BulkHistory"),wait(80),click(700,490),
                       check("bulk.items.1.status","done"),wait(80),click(560,440),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.failed",1),check("bulk.jobs.0.restored",1),
                       shot("bulk-history-undo-failure"),{"type":"restart"},
                       click(1330,36),check("dialog","BulkHistory"),check("bulk.jobs.0.failed",1),
                       wait(80),click(700,490),check("bulk.items.0.status","failed"),
                       wait(80),shot("bulk-history-retry-after-restart"),click(560,440),
                       check("bulk.jobs.0.failed",0),{**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.restored",2),check("total",120),shot("bulk-history-retry-complete"))

    def test_bulk_history_continue_and_job_pages(self):
        started=self.mcp.call("desktop.start",bulk_history=True)
        print(f"History pagination evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(check("bulk.jobs.0.id","paused-fixture"),click(1330,36),check("dialog","BulkHistory"),
                       check("bulk.history_jobs.0","paused-fixture"),check("bulk.jobs.0.paused",True),
                       wait(80),shot("bulk-history-many-groups"),click(700,165),
                       check("bulk.selected_job","paused-fixture"),check("bulk.items.1.status","queued"),
                       wait(80),shot("bulk-history-paused-group"),click(640,440),
                       check("bulk.jobs.0.paused",False),check("bulk.jobs.0.remaining",0),
                       check("bulk.jobs.0.completed",2),shot("bulk-history-continued-group"),click(490,440),
                       check("bulk.selected_job",None),check("bulk.history_loading",False))
        self.scroll_history_to_end()
        self.mcp.batch(shot("bulk-history-before-older"),click(950,840),check("bulk.jobs_offset",20),
                       check("bulk.history_jobs.0","history-05"),check("bulk.history_loading",False),
                       shot("bulk-history-older-groups"),click(490,735),check("bulk.jobs_offset",0),
                       check("bulk.history_jobs.0","paused-fixture"),check("bulk.history_loading",False),
                       shot("bulk-history-newer-groups"))

    def scroll_history_to_end(self):
        self.mcp.batch({"type":"hover","x":960,"y":760},
                       {"type":"scroll","amount":30},{"type":"scroll","amount":30},
                       {"type":"scroll","amount":30},wait(100))

    def test_bulk_history_message_pages_return_to_the_first_row(self):
        started=self.mcp.call("desktop.start",bulk_history=True)
        print(f"History receipt pages evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(check("bulk.jobs.0.id","paused-fixture"),click(1330,36),check("dialog","BulkHistory"),
                       check("bulk.history_jobs.1","paged-fixture"),wait(80),click(700,242),
                       check("bulk.selected_job","paged-fixture"),check("bulk.items.49.position",49),
                       shot("bulk-receipts-first-page"))
        self.scroll_history_to_end()
        self.mcp.batch(click(950,840),check("bulk.items_after",49),check("bulk.items.0.position",50),
                       check("bulk.items.49.position",99),wait(80),shot("bulk-receipts-second-page"))
        self.scroll_history_to_end()
        self.mcp.batch(click(950,840),check("bulk.items_after",99),check("bulk.items.0.position",100),
                       check("bulk.items.19.position",119),wait(80),shot("bulk-receipts-last-page"))
        self.scroll_history_to_end()
        self.mcp.batch(click(510,840),check("bulk.items_after",None),check("bulk.items.0.position",0),
                       wait(80),shot("bulk-receipts-returned-to-first"))

    def test_bulk_crash_resolution_mouse_in_compact_dark(self):
        started=self.mcp.call("desktop.start",persistent=True,mail_actions="slow")
        print(f"Compact group resolution evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(80),click(690,366),
                       check("dark",True),check("preferences_saved",True),key("ctrl+1"),check("tab","Mail"),wait(80))
        self.archive_two_for_recovery()
        self.mcp.batch({"type":"restart","crash":True},check("dark",True),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},check("bulk.jobs.0.uncertain",1),
                       click(1330,36),check("dialog","BulkHistory"),wait(80),click(700,490),
                       check("bulk.items.0.status","uncertain"),{"type":"resize","width":900,"height":640},
                       wait(120),shot("bulk-resolution-dark-compact"),click(280,340),
                       check("bulk.resolving",None,"ne"),wait(80),shot("bulk-confirmation-dark-compact"),
                       click(640,417),check("bulk.jobs.0.uncertain",0),check("bulk.jobs.0.cancelled",1),
                       check("total",119),shot("bulk-resolution-mouse-complete"))

    def test_formatted_reader_and_preferences_survive_graceful_restart(self):
        started=self.mcp.call("desktop.start",persistent=True,html_mail=True)
        print(f"Formatted restart evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(check("html_view_current",True),key("ctrl+comma"),check("tab","Preferences"),
                       wait(80),click(690,366),check("dark",True),check("preferences_saved",True),
                       key("ctrl+1"),check("tab","Mail"),check("html_view_current",True),
                       shot("formatted-before-close"),{"type":"restart"},check("dark",True),
                       check("selected","Styled sign-in sample"),check("html_view_current",True),
                       shot("formatted-after-restart"))

    def test_empty_inbox_survives_graceful_restart_and_retains_archived_mail(self):
        started=self.mcp.call("desktop.start",persistent=True)
        print(f"Empty Inbox restart evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(click(390,245),key("ctrl+a"),check("mail_selection.count",120),
                       check("mail_selection.pending",False),key("Delete"),check("dialog","BulkReview"),
                       check("bulk.review_count",120),key("Return"),check("total",0),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},check("bulk.jobs.0.completed",120),
                       {"type":"restart"},check("page_loaded",True),check("total",0),check("selected",None),
                       shot("empty-inbox-after-restart"),click(82,399),check("folder","Archive"),check("total",120),
                       check("mail_rows.0.subject","A little more room to think"),shot("archived-mail-after-restart"))

    def toggle_html_quotes(self, hidden):
        # Bounds are in parent content coordinates; reset its scroll before use.
        self.mcp.batch({"type":"hover","x":1050,"y":500}, {"type":"scroll","amount":-30},
                       check("html_view_current", True), wait(80))
        x, y, _, height = self.mcp.call("desktop.state")["html_body_bounds"]
        self.mcp.batch(click(int(x+65), int(y+height+34)),
                       check("html_quotes_hidden", hidden), check("html_view_current", True))

    def test_recent_image_heavy_mail_keeps_final_pixels_during_slow_renderer_reopen(self):
        result = self.mcp.call("desktop.start", html_mail=True, html_delay_ms=1200)
        print(f"Visited HTML evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(click(85,355), check("selected","Dispatch update"), check("html_view_current",True),
                       click(1115,376), check("images_allowed",True), check("html_rendered_images",12),
                       check("html_view_current",True), wait(100), shot("visited-dispatch"),
                       click(400,349), check("selected","Delivery update"), check("html_view_current",True))
        if not self.mcp.call("desktop.state")["images_allowed"]:
            self.mcp.batch(click(1115,376),check("images_allowed",True))
        self.mcp.batch(check("html_rendered_images",12),check("html_view_current",True),wait(100),shot("visited-delivery"))
        for index in range(4):
            before = self.mcp.call("desktop.state")["html_cache_hits"]
            self.mcp.batch(click(400,245 if index % 2 == 0 else 349),
                           {**check("html_cache_hits",before+1,"gte"),"timeout_ms":700},
                           {"type":"assert","path":"html_rendered_images","op":"eq","value":12},
                           check("html_view_current",True),shot(f"visited-images-reopen-{index}"))
        self.mcp.batch(check("html_cache_bytes",33554432,"lte"),click(850,500),key("ctrl+a"),
                       check("html_selected_text","Fictional workshop supplies","contains"),
                       key("ctrl+f"),check("focused_input","find-message"),type_text("Quantity"),check("find_count",12))

    def assert_letter_column_pixels(self, directory, name):
        from PIL import Image
        state = self.mcp.call("desktop.state")
        x,y,width,_ = state["html_body_bounds"]
        capture = Image.open(Path(directory)/(name+".webp")).convert("RGB")
        ink = [px for py in range(int(y+16),int(y+40))
               for px in range(int(x),int(x+width))
               if max(capture.getpixel((px,py))) < 100]
        self.assertTrue(ink,"Expected visible letter text below its top padding")
        expected = x + max(0,(width-48*state["reader_size"])/2) + 20
        self.assertAlmostEqual(min(ink),expected,delta=4)

    def test_reading_columns_plain_html_selection_find_and_resize(self):
        started=self.mcp.call("desktop.start",reading_mail=True)
        print(f"Reading column evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(check("selected","Reading style plain letter"),check("reader_text_ready",True),
                       double_click(400,245),check("full_reader",True),wait(120),shot("plain-letter-full"),
                       click(500,410),key("ctrl+a"),check("reader_selected_text","Column marker","contains"),
                       key("ctrl+f"),check("focused_input","find-message"),key("ctrl+a"),type_text("selectable"),check("find_count",1),
                       key("Escape"),check("find_open",False),key("Escape"),check("full_reader",False),
                       double_click(400,349),check("selected","Reading style HTML letter"),check("full_reader",True),
                       check("html_view_current",True),wait(120),shot("html-letter-full"))
        self.assert_letter_column_pixels(started["artifacts"],"html-letter-full")
        state=self.mcp.call("desktop.state")
        x,y,width,_=state["html_body_bounds"]
        self.mcp.batch(click(int(x+width/2),int(y+30)),key("ctrl+a"),
                       check("html_selected_text","Column marker","contains"),
                       key("ctrl+f"),check("focused_input","find-message"),key("ctrl+a"),type_text("selectable"),check("find_count",1),
                       key("Escape"),check("find_open",False),
                       {"type":"resize","width":900,"height":640},check("html_view_current",True),wait(150),shot("html-letter-compact"))
        self.assert_letter_column_pixels(started["artifacts"],"html-letter-compact")
        self.mcp.batch(key("Escape"),check("full_reader",False),click(400,245),
                       check("selected","Reading style plain letter"),check("reader_text_ready",True),shot("plain-letter-compact"))

    def test_conversation_surfaces_follow_each_message_background_without_losing_scroll(self):
        from PIL import Image
        for dark in (False,True):
            started=self.mcp.call("desktop.start",reading_mail=True)
            print(f"Conversation surface evidence: {started['artifacts']}",flush=True)
            if dark:
                self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(100),click(690,366),
                               check("dark",True),key("ctrl+1"),check("tab","Mail"))
            self.mcp.batch(click(400,453),check("conversation_total",2),check("html_view_current",True),
                           check("html_background",[255,255,255,255]),wait(120),shot(f"conversation-white-{dark}"))
            state=self.mcp.call("desktop.state")
            x,y,_,height=state["html_body_visible"]
            capture=Image.open(Path(started["artifacts"])/f"conversation-white-{dark}.webp").convert("RGB")
            self.assertTrue(all(v>=247 for v in capture.getpixel((int(x-8),int(y+min(25,height/2))))))
            self.mcp.batch(click(800,258),check("loaded_message_id","preview-work:Archive:reading-2"),
                           check("html_background",[23,42,58,255]),check("html_view_current",True),wait(120),shot(f"conversation-navy-{dark}"))
            state=self.mcp.call("desktop.state")
            x,y,_,height=state["html_body_visible"]
            capture=Image.open(Path(started["artifacts"])/f"conversation-navy-{dark}.webp").convert("RGB")
            pixel=capture.getpixel((int(x-8),int(y+min(25,height/2))))
            self.assertTrue(all(abs(a-b)<=6 for a,b in zip(pixel,(23,42,58))),pixel)
            for _ in range(2):
                self.mcp.batch(click(800,785),check("loaded_message_id","preview-work:INBOX:reading-3"),
                               check("html_background",[255,255,255,255]),check("html_view_current",True),
                               click(800,258),check("loaded_message_id","preview-work:Archive:reading-2"),
                               check("html_background",[23,42,58,255]),check("html_view_current",True))
            before=self.mcp.call("desktop.state")["conversation_scroll"]
            self.mcp.batch(key("ctrl+r"),check("refreshing",True),check("refreshing",False),
                           check("conversation_scroll",before),check("html_view_current",True),
                           {"type":"resize","width":900,"height":640},check("html_view_current",True),wait(150),shot(f"conversation-surface-compact-{dark}"))

    def test_html_background_matches_document_surround_in_both_themes(self):
        from PIL import Image
        for dark in (False, True):
            result = self.mcp.call("desktop.start", html_mail=True)
            if dark:
                self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(100),click(690,366),check("dark",True),key("ctrl+1"))
            self.mcp.batch(click(85,355),check("selected","Dispatch update"),check("html_view_current",True),
                           check("html_background",[255,255,255,255]),wait(100),shot(f"html-white-surround-{dark}"))
            state = self.mcp.call("desktop.state")
            x,y,w,h = state["html_body_visible"]
            capture = Image.open(Path(result["artifacts"])/f"html-white-surround-{dark}.webp").convert("RGB")
            pixel = capture.getpixel((int(x-20),int(y+30)))
            self.assertTrue(all(v >= 247 for v in pixel),pixel)
            self.mcp.batch(click(85,282),check("folder","INBOX"),check("html_view_current",True),
                           check("html_background",[16,16,16,255]),wait(100),shot(f"html-dark-document-{dark}"))

    def test_conversation_refresh_preserves_scrolled_position(self):
        self.mcp.call("desktop.start",conversation_mail=True)
        self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("Long project review"),
                       check("total",25),key("Escape"),check("conversation_total",25),wait(100),
                       click(1288,194),check("conversation_offset",0),wait(100),
                       {"type":"hover","x":1050,"y":600},{"type":"scroll","amount":12},
                       check("conversation_scroll",400,"gte"),shot("thread-before-refresh"))
        before = self.mcp.call("desktop.state")["conversation_scroll"]
        self.mcp.batch(key("ctrl+r"),check("busy","sync","contains"),check("busy",[],"eq"),
                       check("conversation_scroll",before),wait(100),shot("thread-after-refresh"))

    def test_sender_copy_icons_and_list_selection_icon(self):
        self.mcp.call("desktop.start",html_mail=True)
        self.mcp.batch(check("html_view_current",True),click(574,155),check("mail_selection.mode",True),
                       shot("square-select-active"),click(574,155),check("mail_selection.mode",False),
                       click(740,243),check("dialog","Sender"),shot("sender-copy-icons"),
                       click(958,490),key("Escape"),check("dialog",None),key("ctrl+k"),
                       check("focused_input","search"),key("ctrl+v"),check("query","support@example.test"),
                       key("ctrl+a"),key("BackSpace"),check("total",124),key("Escape"),
                       check("html_view_current",True),click(740,243),check("dialog","Sender"),
                       click(958,570),key("Escape"),key("ctrl+k"),check("focused_input","search"),
                       key("ctrl+v"),check("query","example.test"),shot("sender-domain-copied"))

    def test_html_loading_keeps_body_origin_stable_with_css_images_and_horizontal_controls(self):
        result = self.mcp.call("desktop.start", html_mail=True, html_delay_ms=1200)
        print(f"HTML layout evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("html_view_current", True), click(85,398),
                       check("selected", "CSS background report"), check("html_body_bounds", None, "ne"),
                       check("html_ready", False), check("images_allowed", False), shot("html-css-loading"))
        loading = self.mcp.call("desktop.state")
        self.mcp.batch(check("html_view_current",True), check("html_width",1000,"gte"),
                       wait(120), shot("html-css-ready"))
        ready = self.mcp.call("desktop.state")
        self.assertEqual(loading["html_body_bounds"][1], ready["html_body_bounds"][1], "Rendering must not insert controls above the body")
        x, y, width, height = ready["html_body_visible"]
        self.mcp.batch(drag(int(x+100), int(y+height-8), int(x+width-4), int(y+height-8)),
                       check("html_pan",100,"gte"), check("html_view_current",True), wait(100), shot("html-css-panned"),
                       click(int(x+140),int(y+35)), key("Left"), check("html_view_current",True),
                       key("ctrl+f"), check("focused_input","find-message"), type_text("Last report column"),
                       check("find_count",1), check("html_view_current",True))
        pan = self.mcp.call("desktop.state")["html_pan"]
        self.mcp.batch(key("Left"), key("Right"), wait(80), check("html_pan",pan), shot("html-css-find"),
                       key("Escape"), check("find_open",False))

    def test_html_image_arrival_keeps_the_reading_position(self):
        result = self.mcp.call("desktop.start", html_mail=True, image_delay_ms=2000)
        print(f"Image reflow evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(click(85,440), check("selected","Delayed illustrated report"),
                       check("html_view_current",True), click(1115,376), check("images_allowed",True),
                       check("html_view_current",True), check("html_loaded_images",0),
                       click(850,450), key("Next"), key("Next"), check("html_scroll",500,"gte"),
                       check("html_view_current",True), shot("html-before-image-arrival"))
        before = self.mcp.call("desktop.state")
        self.mcp.batch(check("html_loaded_images",2), check("html_height",before["html_height"]+400,"gte"),
                       check("html_view_current",True), wait(120), shot("html-after-image-arrival"))
        after = self.mcp.call("desktop.state")
        self.assertAlmostEqual(after["html_scroll"]-before["html_scroll"],
                               after["html_height"]-before["html_height"], delta=2.,
                               msg="Images above the viewport must not displace the paragraph being read")

    def test_html_compact_preview_preserves_readable_body_and_all_attachment_controls(self):
        result = self.mcp.call("desktop.start", width=900, height=640)
        print(f"Compact HTML evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(key("ctrl+comma"), check("tab","Preferences"), wait(100), click(563,366), check("dark",True),
                       key("ctrl+1"), check("tab","Mail"), key("ctrl+k"), check("focused_input","search"),
                       type_text("prototype"), check("total",1), key("Escape"), check("html_view_current",True),
                       check("attachment_count",4), wait(120), shot("html-compact-reading-space"))
        state = self.mcp.call("desktop.state")
        self.assertGreaterEqual(state["html_body_visible"][3], min(120.,state["html_body_bounds"][3]))
        self.mcp.batch(key("f"), check("dialog","Compose"), check("draft_attachments.3.name",None,"ne"),
                       key("Escape"), check("dialog",None))

    def test_html_image_arrival_keeps_compact_dark_find_and_reading_position(self):
        result = self.mcp.call("desktop.start", width=900, height=640, html_mail=True, image_delay_ms=3000)
        print(f"Compact image reflow evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(key("ctrl+comma"), check("tab","Preferences"), wait(100), click(563,366), check("dark",True),
                       key("ctrl+1"), check("tab","Mail"), click(85,440), check("selected","Delayed illustrated report"),
                       check("html_view_current",True), click(807,296), wait(80), click(800,328), check("images_allowed",True),
                       check("html_view_current",True), key("ctrl+f"), check("focused_input","find-message"),
                       type_text("Reading paragraph 30"), check("find_count",1), check("html_scroll",500,"gte"),
                       check("html_view_current",True), check("html_loaded_images",0), shot("html-dark-before-image-arrival"))
        before = self.mcp.call("desktop.state")
        self.mcp.batch(check("html_loaded_images",2), check("html_height",before["html_height"]+400,"gte"),
                       check("html_view_current",True), check("find_count",1), wait(120), shot("html-dark-after-image-arrival"))
        after = self.mcp.call("desktop.state")
        self.assertAlmostEqual(after["html_scroll"]-before["html_scroll"],400.,delta=2.)
        self.mcp.batch(key("Escape"), check("find_open",False), click(700,450), key("Home"),
                       check("html_scroll",0), check("html_view_current",True), shot("html-images-at-start"))

    def test_html_late_images_do_not_scroll_a_different_message(self):
        result = self.mcp.call("desktop.start", html_mail=True, image_delay_ms=2000)
        print(f"Late image navigation evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(click(85,440), check("selected","Delayed illustrated report"), check("html_view_current",True),
                       click(1115,376), check("images_allowed",True), check("remote_image_pending",2),
                       click(850,450), key("Next"), check("html_scroll",100,"gte"),
                       click(85,278), check("selected","Styled sign-in sample"), check("html_view_current",True),
                       check("remote_image_pending",0), check("remote_image_cached",2,"gte"),
                       check("html_scroll",0), check("images_allowed",False), check("html_view_current",True),
                       wait(120), shot("html-navigation-after-late-images"))

    def test_html_render_failure_has_a_working_native_retry(self):
        result = self.mcp.call("desktop.start", html_mail=True, html_failure_once=True)
        print(f"HTML recovery evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("html_error",None,"ne"), wait(100), shot("html-render-failure"),
                       click(733,468), check("html_error",None), check("html_view_current",True),
                       check("selected","Styled sign-in sample"), wait(100), shot("html-render-retried"),
                       click(772,320), check("html_formatted",False), check("reader_text_ready",True),
                       click(685,320), check("html_view_current",True), check("html_error",None))

    def test_refresh_icons_in_mail_calendar_and_compact_dark(self):
        self.mcp.batch(check("reader_text_ready", True), wait(150), shot("refresh-mail-light"),
                       click(1400,36), check("refreshing",True), {"type":"hover","x":1100,"y":35},
                       shot("refresh-mail-light-busy"), key("ctrl+2"), check("tab","Calendar"),
                       wait(120), shot("refresh-calendar-light"), check("busy",[]),
                       key("ctrl+comma"), check("tab","Preferences"), wait(80),
                       click(690,366), check("dark",True), key("ctrl+1"), check("tab","Mail"),
                       {"type":"resize","width":900,"height":640}, wait(150), shot("refresh-mail-dark-compact"),
                       click(852,34), check("refreshing",True), {"type":"hover","x":750,"y":35},
                       shot("refresh-mail-dark-compact-busy"), key("ctrl+2"), check("tab","Calendar"),
                       wait(150), shot("refresh-calendar-dark-compact"), check("busy",[]))

    def test_refresh_icons_and_html_at_larger_interface_scale(self):
        result = self.mcp.call("desktop.start", html_mail=True)
        print(f"Scaled refresh evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("html_view_current",True), key("ctrl+comma"), check("tab","Preferences"),
                       wait(120), click(1145,623), wait(80), shot("interface-scale-menu"),
                       click(1140,509), check("interface_scale",120), check("preferences_saved",True),
                       key("ctrl+1"), check("tab","Mail"), check("html_view_current",True),
                       wait(150), shot("refresh-html-scaled"), click(1392,43), check("refreshing",True),
                       {"type":"hover","x":1000,"y":43}, shot("refresh-scaled-busy"),
                       key("ctrl+2"), check("tab","Calendar"), wait(120), shot("refresh-calendar-scaled"))

    def test_html_prepared_neighbors_rapid_navigation_and_resize(self):
        result = self.mcp.call("desktop.start", html_mail=True)
        print(f"HTML preparation evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("html_view_current",True))
        state = self.mcp.call("desktop.state")
        second = state["mail_rows"][1]["id"]
        self.mcp.batch(check("html_cache_ids",second,"contains"),
                       click(400,351), check("selected","Mislabeled XHTML request"),
                       check("html_cache_hits",state["html_cache_hits"] + 1,"gte"),
                       check("html_view_current",True), wait(120), shot("html-prepared-neighbor"),
                       click(400,452), check("selected","Escaped HTML request"),
                       click(400,558), check("selected","Long formatted letter"),
                       click(400,245), check("selected","Styled sign-in sample"),
                       click(400,558),
                       check("selected","Long formatted letter"), check("html_view_current",True),
                       check("html_height",6000,"gte"), wait(100),
                       click(850,440), key("ctrl+a"), check("html_selected_text","Last visible paragraph.","contains"),
                       key("End"), check("html_scroll",5000,"gte"), check("html_view_current",True),
                       wait(120), shot("html-current-bottom"), key("Home"), check("html_scroll",0),
                       drag(616,500,750,500), check("reader_split",.4,"gte"), check("html_view_current",True),
                       wait(120), shot("html-current-narrower"),
                       {"type":"resize","width":900,"height":640}, check("window_size",[900,640]),
                       check("html_view_current",True), wait(120), shot("html-current-compact"),
                       check("html_error",None), check("html_cache_bytes",33554432,"lte"))

    def test_print_formatted_plain_and_long_messages_to_real_browser_pdfs(self):
        result = self.mcp.call("desktop.start", html_mail=True, print_browser="pdf")
        print(f"Print evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("html_ready", True), wait(150), shot("print-reader-control"),
                       click(879,830), check("print_revision", 1),
                       {"type":"print_output", "text":"Styled sign-in sample", "name":"print-styled"},
                       check("print_pending",False), click(772,320), check("html_formatted",False),
                       key("ctrl+p"), check("print_revision",2),
                       {"type":"print_output", "count":2, "text":"This is the plain text alternative.", "name":"print-plain"},
                       click(400,558), check("selected","Long formatted letter"), check("html_ready",True),
                       key("ctrl+p"), check("print_revision",3),
                       {"type":"print_output", "count":3, "text":"Last visible paragraph.", "pages":3, "name":"print-long"},
                       check("print_pending",False), shot("print-returned-to-mail"))

    def test_print_pending_navigation_and_failure_retry_keep_the_original_target(self):
        result = self.mcp.call("desktop.start", html_mail=True, print_browser="pdf", mail_actions="fail")
        print(f"Print evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("html_ready",True), key("ctrl+p"), check("print_pending",True),
                       click(400,351), check("selected","Mislabeled XHTML request"),
                       check("print_pending",False), check("notice","Try Print again","contains"),
                       check("html_ready",True), key("ctrl+p"), check("print_pending",True),
                       click(400,558), check("selected","Long formatted letter"),
                       {"type":"print_output", "text":"Mislabeled XHTML request", "name":"print-original-target"},
                       check("print_pending",False), check("selected","Long formatted letter"), check("notice","restored","contains"))

    def test_print_shortcuts_remap_disable_and_text_input_isolation(self):
        self.mcp.call("desktop.start", print_browser="fail")
        self.mcp.batch(key("ctrl+k"), check("focused_input","search"), key("ctrl+p"), wait(80),
                       check("print_revision",0), key("Escape"), key("ctrl+p"), check("print_revision",1),
                       check("print_pending",False), check("notice","isolated print browser","contains"),
                       click(1400,895), check("notice",None), key("ctrl+comma"), check("tab","Preferences"),
                       click(645,156), check("settings_tab","Shortcuts"), {"type":"hover","x":1110,"y":690},
                       {"type":"scroll","amount":30}, wait(100), click(905,780), key("F6"),
                       check("shortcuts.Print","F6"), click(1070,780), key("alt+p"), check("shortcut_secondary.Print","Alt+P"),
                       check("preferences_saved",True), shot("print-shortcuts"),
                       key("ctrl+1"), check("tab","Mail"), key("ctrl+p"), wait(80), check("print_revision",1),
                       key("F6"), check("print_revision",2), check("print_pending",False),
                       click(1400,895), check("notice",None), key("alt+p"), check("print_revision",3), check("print_pending",False),
                       click(1400,895), check("notice",None), key("ctrl+comma"), check("tab","Preferences"),
                       {"type":"hover","x":1110,"y":690}, {"type":"scroll","amount":30}, wait(100),
                       click(988,780), check("shortcuts.Print",""), click(1157,780), check("shortcut_secondary.Print",""),
                       key("ctrl+1"), check("tab","Mail"), key("F6"), key("alt+p"), wait(80), check("print_revision",3))

    def test_print_cancel_and_compact_dark_attachment_layout(self):
        result = self.mcp.call("desktop.start", print_browser="dialog")
        print(f"Print evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(key("ctrl+comma"), check("tab","Preferences"), wait(80), click(690,366), check("dark",True),
                       key("ctrl+1"), check("tab","Mail"), key("ctrl+k"), check("focused_input","search"),
                       type_text("prototype"), check("total",1), key("Escape"), check("html_ready",True),
                       {"type":"resize","width":900,"height":640}, wait(250), shot("print-compact-dark-controls"),
                       key("ctrl+p"), check("print_revision",1), check("print_pending",False), wait(1000),
                       {"type":"browser_screenshot","name":"print-native-dialog"}, {"type":"cancel_print"},
                       check("total",1), check("dialog",None), shot("print-cancel-return"),
                       key("ctrl+p"), check("print_revision",2), check("print_pending",False), wait(700),
                       {"type":"cancel_print"}, check("notice",None), check("total",1))
        self.assertEqual(list((Path(result["artifacts"])/"printed").glob("*.pdf")), [])

    def test_forward_mouse_preserves_attachments_and_reopens_as_an_independent_draft(self):
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("prototype"),
                       check("total", 1), key("Escape"), check("html_ready", True), wait(100),
                       shot("forward-reader-actions"), click(835,784), check("dialog", "Compose"),
                       check("draft_forward", True), check("draft_forward_html", True), check("fields.subject", "Fwd: Re: A few thoughts on the prototype"),
                       check("fields.to", ""), check("fields.cc", ""), check("fields.bcc", ""),
                       check("draft_in_reply_to", None), check("draft_attachments.3.name", "review-checklist.txt"),
                       check("editor", "Can you send the updated prototype?", "contains"),
                       check("focused_input", "to"), type_text("reviewer@example.test"), wait(250),
                       shot("forward-composer-files"), key("Escape"), check("dialog", None), check("draft_count", 1),
                       click(98,517), check("dialog", "Compose"), check("fields.to", "reviewer@example.test"),
                       check("draft_forward", True), check("draft_forward_html", True), check("draft_attachments.3.name", "review-checklist.txt"),
                       shot("forward-reopened"))

    def test_forward_preparation_keeps_navigation_available_and_does_not_replace_newer_edits(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(key("f"), check("forward_pending", True), check("dialog", None),
                       key("f"), click(400,350), check("selected", "Your weekly workspace digest"),
                       key("c"), check("dialog", "Compose"), wait(100),
                       click(650,362), type_text("A different draft"),
                       {**check("forward_pending", False), "timeout_ms":5000},
                       check("dialog", "Compose"), check("fields.subject", "A different draft"),
                       check("draft_forward", False), check("notice", "Forward saved in Drafts.", "contains"),
                       check("draft_count", 2), shot("forward-pending-preserves-editor"),
                       key("Escape"), check("dialog", None), check("draft_count", 2))

    def test_forward_failure_retry_and_compact_dark_layout(self):
        self.mcp.call("desktop.start", mail_actions="fail")
        self.mcp.batch(key("f"), check("forward_pending", True),
                       {**check("forward_pending", False), "timeout_ms":5000}, check("dialog", None),
                       check("draft_count", 0), check("notice", "Try Forward again", "contains"),
                       key("f"), check("forward_pending", True),
                       {**check("dialog", "Compose"), "timeout_ms":5000}, check("draft_count", 1),
                       check("fields.subject", "Fwd: A little more room to think"), key("Escape"), check("dialog", None),
                       key("ctrl+comma"), check("tab", "Preferences"), wait(80), click(690,366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), {"type":"resize", "width":900, "height":640}, wait(150),
                       click(98,517), check("dialog", "Compose"), check("draft_forward", True),
                       check("fields.to", ""), shot("forward-dark-compact"))

    def test_forward_targets_the_expanded_message_in_a_conversation(self):
        self.mcp.call("desktop.start", conversation_mail=True)
        self.mcp.batch(check("conversation_total",3), check("loaded_message_id","preview-work:INBOX:launch-2"),
                       wait(100), click(800,344), check("loaded_message_id","preview-work:Sent:launch-1"),
                       check("selected_id","preview-work:INBOX:launch-2"), check("attachment_count",1),
                       key("f"), check("dialog","Compose"), check("fields.subject","Launch schedule","contains"),
                       check("draft_attachments.0.size",1,"gte"), check("fields.to",""), check("draft_in_reply_to",None),
                       shot("forward-conversation-target"))

    def test_forward_shortcuts_remap_disable_and_text_input_isolation(self):
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("f"), check("dialog", None),
                       key("ctrl+a"), key("BackSpace"), check("total",120), key("Escape"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(645,156), check("settings_tab", "Shortcuts"),
                       {"type":"hover","x":1110,"y":690}, {"type":"scroll","amount":30}, wait(100),
                       click(905,720), key("F4"), check("shortcuts.Forward", "F4"),
                       click(1070,720), key("alt+f"), check("shortcut_secondary.Forward", "Alt+F"), check("preferences_saved",True),
                       key("ctrl+1"), check("tab", "Mail"), key("ctrl+k"), check("focused_input","search"),
                       key("F4"), key("alt+f"), check("dialog",None), check("query", "f"),
                       key("ctrl+a"), key("BackSpace"),
                       check("query",""), check("selected","A little more room to think"), key("Escape"), wait(80),
                       key("F4"), check("dialog","Compose"),
                       key("Escape"), check("dialog",None), key("alt+f"), check("dialog","Compose"),
                       key("Escape"), check("dialog",None), check("notice", "Draft saved.", "contains"),
                       click(1400,895), check("notice",None), key("ctrl+comma"), check("tab","Preferences"),
                       click(645,156), check("settings_tab","Shortcuts"), {"type":"hover","x":1110,"y":690},
                       {"type":"scroll","amount":30}, wait(100), click(988,720), check("shortcuts.Forward",""),
                       click(1157,720), check("shortcut_secondary.Forward",""), check("preferences_saved",True),
                       key("ctrl+1"), check("tab","Mail"), key("F4"), key("alt+f"), key("f"), check("dialog",None),
                       check("draft_count",2))

    def test_find_formatted_message_navigation_and_keyboard_isolation(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(click(400,558), check("selected", "Long formatted letter"), check("html_ready", True),
                       key("ctrl+f"), check("find_open", True), check("focused_input", "find-message"),
                       type_text("Paragraph"), check("find_count", 201), check("find_pending", False),
                       check("find_active", 0), shot("find-html-first"),
                       click(1261,158), check("find_match_case", True), check("find_count", 200),
                       click(1261,158), check("find_match_case", False), check("find_count", 201),
                       click(1344,158), check("find_active", 1), click(1300,158), check("find_active", 0),
                       key("Return"), check("find_active", 1), key("shift+Return"), check("find_active", 0),
                       key("shift+Return"), check("find_active", 200), check("html_scroll", 5000, "gte"),
                       shot("find-html-last"), key("ctrl+d"), check("total", 124),
                       key("Escape"), check("find_open", False), check("full_reader", False),
                       double_click(400,555), check("full_reader", True), key("ctrl+f"),
                       check("find_open", True), check("find_count", 201), key("Escape"),
                       check("find_open", False), check("full_reader", True), key("Escape"), check("full_reader", False))

    def test_find_plain_message_and_switching_messages_discards_old_results(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(click(400,558), check("selected", "Long formatted letter"), check("html_ready", True),
                       click(770,320), check("html_formatted", False), check("reader_text_ready", True),
                       key("ctrl+f"), check("focused_input", "find-message"), type_text("Paragraph"),
                       check("find_count", 201), check("find_pending", False), check("find_error", None),
                       shot("find-plain-first"), key("shift+Return"), check("find_active", 200),
                       shot("find-plain-last"), click(400,450), check("selected", "Escaped HTML request"), check("focused_input", None),
                       check("find_count", 0), check("find_pending", False), key("ctrl+f"),
                       check("focused_input", "find-message"), key("ctrl+a"), type_text("Readable content"),
                       check("find_count", 1), check("find_pending", False), shot("find-new-message"),
                       key("ctrl+a"), type_text("[not.*present]"), check("find_count", 0),
                       check("find_pending", False), check("find_error", None), shot("find-no-results"),
                       key("Escape"), check("find_open", False))

    def test_find_wide_message_reveals_match_in_dark_compact_reader(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(690,366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), click(100,536), check("folder", "Projects"),
                       check("selected", "Wide HTML report"), check("html_ready", True),
                       click(828,100), check("find_open", True), check("focused_input", "find-message"),
                       type_text("Right report column"), check("find_count", 1), shot("find-wide-dark"),
                       {"type":"resize","width":900,"height":640}, check("window_size", [900,640]),
                       check("html_pan", 100, "gte"), check("find_count", 1), {"type":"hover","x":230,"y":40}, wait(100), shot("find-wide-dark-compact"),
                       double_click(380,245), check("full_reader", True), check("find_count", 1),
                       shot("find-wide-dark-full"), key("Escape"), check("find_open", False),
                       check("full_reader", True), key("Escape"), check("full_reader", False))

    def test_find_remap_secondary_binding_disable_and_mouse_close(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(645,156), check("settings_tab", "Shortcuts"),
                       {"type":"hover","x":1200,"y":700}, {"type":"scroll","amount":30}, wait(150), shot("find-shortcut-settings"),
                       click(905,660), key("F3"), check("shortcuts.Find", "F3"), check("preferences_saved", True),
                       click(1070,660), key("ctrl+f"), check("shortcut_secondary.Find", "Mod+F"), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), key("F3"), check("find_open", True), check("focused_input", "find-message"),
                       type_text("conversation"), check("find_query", "conversation"), check("find_pending", False),
                       shot("find-remapped-open"), click(1388,158), check("find_open", False),
                       key("ctrl+f"), check("find_open", True), key("Escape"), check("find_open", False),
                       key("ctrl+comma"), check("tab", "Preferences"),
                       {"type":"hover","x":1200,"y":700}, {"type":"scroll","amount":30}, wait(100),
                       click(988,660), check("shortcuts.Find", ""), click(1157,660), check("shortcut_secondary.Find", ""),
                       check("preferences_saved", True), key("ctrl+1"), check("tab", "Mail"),
                       key("F3"), wait(80), check("find_open", False), key("ctrl+f"), wait(80), check("find_open", False))

    def test_find_respects_html_and_plain_quoted_history(self):
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("prototype"), check("total", 1),
                       key("Escape"), check("html_ready", True), check("html_quotes_hidden", True),
                       key("ctrl+f"), check("focused_input", "find-message"), type_text("updated"),
                       check("find_pending", False), check("find_count", 0), key("Escape"), check("find_open", False))
        self.toggle_html_quotes(False)
        self.mcp.batch(key("ctrl+f"), check("focused_input", "find-message"),
                       check("find_count", 1), shot("find-html-quote"), key("Escape"), check("find_open", False))
        self.toggle_html_quotes(True)
        self.mcp.batch(key("ctrl+f"), check("find_open", True), check("focused_input", "find-message"), check("find_pending", False),
                       check("find_count", 0), key("Escape"), check("find_open", False), wait(80),
                       {"type":"hover","x":1050,"y":500}, {"type":"scroll","amount":-30}, wait(100), click(770,320), check("html_formatted", False), check("reader_text_ready", True),
                       key("ctrl+f"), check("find_open", True), check("focused_input", "find-message"), check("find_pending", False), check("find_count", 0), key("Escape"), check("find_open", False), wait(80),
                       click(740,419), check("expanded_replies", [0]), key("ctrl+f"), check("find_count", 1),
                       shot("find-plain-quote"), key("Escape"), check("find_open", False))

    def test_html_styled_message_plain_alternative_and_raw_xhtml(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(check("selected", "Styled sign-in sample"), check("html_ready", True),
                       check("html_error", None), check("html_formatted", True),
                       check("images_allowed", False), wait(200), shot("html-styled-blocked"),
                       click(772, 320), check("html_formatted", False), check("reader_text_ready", True),
                       shot("html-plain-alternative"), click(685, 320), check("html_ready", True),
                       click(400, 351), check("selected", "Mislabeled XHTML request"), check("html_ready", True),
                       wait(100), shot("html-mislabeled-xhtml"), click(800, 402), key("ctrl+a"),
                       check("html_selected_text", "SAMPLE-ONLY", "contains"), key("ctrl+c"),
                       key("ctrl+k"), check("focused_input", "search"), key("ctrl+v"),
                       check("query", "SAMPLE-ONLY", "contains"), key("ctrl+a"), key("BackSpace"),
                       check("total", 124), key("Escape"), click(400, 452),
                       check("selected", "Escaped HTML request"), check("html_ready", True), wait(100),
                       shot("html-escaped-tags"))

    def test_html_selection_scrolling_and_pending_mail_actions(self):
        self.mcp.call("desktop.start", html_mail=True, mail_actions="slow")
        self.mcp.batch(check("html_ready", True), click(400, 558), check("selected", "Long formatted letter"),
                       check("html_ready", True), check("html_height", 6000, "gte"),
                       wait(100), {"type": "hover", "x": 1100, "y": 600}, {"type": "scroll", "amount": 12},
                       check("html_scroll", 1, "gte"), wait(100), shot("html-long-scrolled"),
                       click(850, 480), key("ctrl+a"), check("html_selected_text", "Last visible paragraph.", "contains"),
                       shot("html-long-selected"), click(784, 100), check("starred", True),
                       check("mail_pending", 1, "gte"), check("html_ready", True),
                       click(652, 100), check("action_toast.label", "Archived 1 message"), check("mail_pending", 1, "gte"),
                       shot("html-archive-immediate-toast"))

    def test_html_dark_full_reader_and_compact_layout(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), wait(80),
                       click(690, 366), check("dark", True), key("ctrl+1"), check("tab", "Mail"),
                       check("html_ready", True), wait(120), shot("html-dark-preview"),
                       double_click(400, 245), check("full_reader", True), check("html_ready", True),
                       wait(150), shot("html-dark-full-reader"),
                       {"type": "hover", "x": 1100, "y": 600}, {"type": "scroll", "amount": 12}, wait(100),
                       shot("html-dark-full-bottom"), click(150, 598), check("html_link", "https://example.test/help"),
                       {"type": "resize", "width": 900, "height": 640}, check("window_size", [900,640]),
                       check("html_error", None), wait(150), shot("html-dark-compact-full"),
                       key("Escape"), check("full_reader", False), wait(150), shot("html-dark-compact-preview"),
                       click(807,296), wait(100), shot("html-compact-image-menu"),
                       click(800,328), check("images_allowed", True), check("html_loaded_images", 1),
                       wait(100), shot("html-compact-images-allowed"),
                       {"type": "hover", "x": 750, "y": 460}, {"type": "scroll", "amount": 8},
                       check("html_scroll", 1, "gte"), wait(100), shot("html-dark-compact-body"))

    def test_html_wide_table_can_scroll_horizontally_and_select_its_right_column(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(click(100, 536), check("folder", "Projects"), check("selected", "Wide HTML report"),
                       check("html_view_current", True), check("html_width", 1000, "gte"), wait(100), shot("html-wide-table"))
        x, y, width, height = self.mcp.call("desktop.state")["html_body_visible"]
        self.mcp.batch(drag(int(x+150),int(y+height-8),int(x+width-4),int(y+height-8)),
                       check("html_pan", 100, "gte"), check("html_view_current",True), wait(100), shot("html-wide-table-right"),
                       drag(904,int(y+32),1061,int(y+32)), check("html_selected_text", "Right report column"),
                       key("ctrl+a"), check("html_selected_text", "Right report column", "contains"),
                       shot("html-wide-table-selection"))

    def test_html_keyboard_scroll_is_scoped_to_the_focused_reader(self):
        self.mcp.call("desktop.start", html_mail=True)
        self.mcp.batch(click(400,558), check("selected", "Long formatted letter"), check("html_ready", True),
                       wait(100), click(850,440), key("Next"), check("html_scroll", 1, "gte"),
                       key("Down"), check("selected", "Long formatted letter"), key("End"),
                       check("html_scroll", 5000, "gte"), wait(100), shot("html-keyboard-bottom"),
                       key("Home"), check("html_scroll", 0), check("selected", "Long formatted letter"),
                       click(400,555), key("Up"), check("selected", "Escaped HTML request"),
                       check("html_ready", True), wait(100), shot("html-inbox-arrows-after-reader-focus"))

    def test_search_best_match_beats_newer_mail_and_sort_can_be_overridden(self):
        self.mcp.call("desktop.start", search_mail=True)
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("test"),
                       check("sort", "Relevance"), check("selected", "Quick note"),
                       check("mail_rows.0.subject", "Quick note"), shot("search-exact-body-first"),
                       click(531, 100), wait(100), shot("search-sort-menu"), click(531, 155),
                       check("sort", "Newest"), check("selected", "Testing checklist"), check("focused_input", None),
                       key("ctrl+k"), check("focused_input", "search"), wait(80), key("ctrl+a"), type_text("testing"), check("query", "testing"), check("sort", "Newest"),
                       key("ctrl+a"), key("BackSpace"), check("query", ""), check("sort", "Newest"),
                       type_text("test"), check("sort", "Relevance"), check("selected", "Quick note"),
                       key("Escape"), key("ctrl+r"), check("refreshing", True),
                       check("refreshing", False), check("selected", "Quick note"), shot("search-relevance-after-sync"))

    def test_move_library_matches_accents_and_fast_typo_enter(self):
        self.mcp.call("desktop.start", search_mail=True)
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("test"),
                       check("selected", "Quick note"), key("Escape"), key("m"),
                       check("focused_input", "folder-search"), type_text("cafe"),
                       check("move_enter_destination", "Café"), shot("move-accent-match-highlight"),
                       key("Return"), check("dialog", None), check("mail_pending", 0),
                       click(100, 617), check("folder", "Café"), check("total", 5),
                       key("ctrl+k"), check("focused_input", "search"), key("ctrl+a"), key("BackSpace"),
                       check("query", ""), check("total", 1), check("selected", "Quick note"), key("Escape"),
                       check("focused_input", None),
                       click(85, 115), check("folder", "INBOX"), check("query", ""),
                       check("total", 123), check("selected", None, "ne"))
        subject = self.selected_mail_subject()
        self.mcp.batch(key("m"), check("dialog", "Move"), check("focused_input", "folder-search"), type_text("archvie"), key("Return"),
                       check("dialog", None), check("mail_pending", 0),
                       click(85, 398), check("folder", "Archive"), check("total", 1),
                       check("selected", subject), shot("move-transposition-enter-result"))

    def test_background_mail_arrives_without_refresh_and_manual_clicks_queue(self):
        self.mcp.call("desktop.start", background_sync=True)
        self.mcp.batch(check("background_sync", True), check("refreshing", False),
                       check("mail_check_seconds", 15), shot("background-sync-refresh-idle"),
                       click(1400, 36), click(1400, 36), key("ctrl+r"),
                       check("refreshing", True), check("background_sync", True),
                       check("sync_round", 1), check("total", 121),
                       check("background_sync", False), check("refreshing", True),
                       shot("manual-refresh-queued-after-background"),
                       {**check("sync_round", 2), "timeout_ms": 5000}, check("refreshing", False),
                       check("total", 121), click(420, 247),
                       check("selected", "New mail from the background"), shot("background-arrival-readable"))

    def open_notification_preferences(self, compact=False):
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),
                       click(650 if compact else 1150,88),type_text("notifications"),
                       check("settings_matches",["Notifications"]),click(450,289),
                       check("settings_group","Notifications"),shot("notification-preferences"))

    def test_notifications_preferences_privacy_independent_sound_and_restart(self):
        result=self.mcp.call("desktop.start",persistent=True)
        print(f"Notification preferences evidence: {result['artifacts']}",flush=True)
        enabled={"popups":True,"sound":True,"show_details":True}
        self.mcp.batch(check("notifications.settings",enabled),check("notifications.requested",0))
        self.open_notification_preferences()
        self.mcp.batch(click(350,499),check("notifications.sent",1),check("notifications.testing",False),
                       check("notifications.last.popups",True),check("notifications.last.sound",True),
                       check("notifications.last.body","Your notification settings are working."),
                       click(288,452),check("notifications.settings.show_details",False),
                       click(350,499),check("notifications.sent",2),
                       check("notifications.last.title","New email"),
                       check("notifications.last.body","You have a new message in your Inbox."),
                       click(288,374),check("notifications.settings.popups",False),
                       click(350,499),check("notifications.sent",3),
                       check("notifications.last.popups",False),check("notifications.last.sound",True),
                       click(288,413),check("notifications.settings.sound",False),
                       click(350,499),check("notifications.requested",3),check("notifications.testing",False),
                       check("preferences_saved",True),shot("notifications-muted"),
                       {"type":"restart"},check("notifications.settings",{"popups":False,"sound":False,"show_details":False}))
        self.open_notification_preferences()
        self.mcp.batch(click(288,374),check("notifications.settings.popups",True),
                       click(350,499),check("notifications.sent",1),check("notifications.last.sound",False),
                       check("preferences_saved",True),shot("notifications-popup-only"))

    def test_notifications_new_arrivals_once_through_sync_read_flag_and_restart(self):
        result=self.mcp.call("desktop.start",persistent=True,background_sync=True)
        print(f"Notification arrival evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(check("background_sync",True),check("notifications.requested",0),
                       check("total",121),check("notifications.sent",1),
                       check("notifications.last.body","New mail from the background"),
                       click(1400,36),{**check("sync_round",2),"timeout_ms":5000},
                       check("refreshing",False),check("notifications.sent",1),
                       click(420,247),check("selected","New mail from the background"),
                       key("s"),check("mail_pending",0),key("u"),check("mail_pending",0),
                       check("notifications.sent",1),shot("notification-arrival-once"),
                       {"type":"restart"},{**check("sync_round",3),"timeout_ms":5000},
                       check("background_sync",False),check("total",121),check("notifications.requested",0),
                       check("notifications.sent",0),shot("notification-restart-no-repeat"))

    def test_notifications_slow_failure_keeps_navigation_available_and_retries(self):
        result=self.mcp.call("desktop.start",notification_delivery="fail-once")
        print(f"Notification recovery evidence: {result['artifacts']}",flush=True)
        self.open_notification_preferences()
        self.mcp.batch(click(350,499),check("notifications.testing",True),
                       key("ctrl+2"),check("tab","Calendar"),check("notifications.testing",True),
                       key("ctrl+1"),check("tab","Mail"),click(420,350),
                       check("selected","Your weekly workspace digest"),
                       check("notifications.error","Fixture notification service unavailable","contains"),
                       check("notifications.testing",False),check("notifications.sent",0),
                       key("ctrl+comma"),check("tab","Preferences"),shot("notification-service-error"),
                       click(350,499),check("notifications.testing",True),
                       check("notifications.sent",1),check("notifications.error",None),
                       check("notifications.testing",False),shot("notification-service-recovered"))

    def test_notifications_compact_dark_settings_and_test(self):
        result=self.mcp.call("desktop.start",width=900,height=640)
        print(f"Compact notifications evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),click(563,366),check("dark",True))
        self.open_notification_preferences(compact=True)
        self.mcp.batch(click(350,499),check("notifications.sent",1),
                       check("notifications.testing",False),check("notifications.error",None),
                       shot("notifications-compact-dark"))

    def test_desktop_badge_tracks_read_changes_after_switching_folders(self):
        self.mcp.call("desktop.start", desktop_badges=True, mail_actions="slow")
        self.mcp.batch(check("unread", True), check("desktop_badge.visible", True))
        initial = self.mcp.call("desktop.state")
        count = initial["desktop_badge"]["count"]
        self.mcp.batch(click(740,100), check("mail_pending",1), check("desktop_badge.count",count-1),
                       click(85,398), check("folder","Archive"),
                       check("count_observed_ids",initial["selected_id"],"contains"),check("mail_pending",1),
                       check("desktop_badge.count",count-1),
                       {**check("mail_pending",0), "timeout_ms":5000}, check("desktop_badge.count",count-1),
                       shot("badge-read-after-folder-navigation"))
        history = self.mcp.call("desktop.state")["desktop_badge"]["history"]
        self.assertTrue(all(n == count-1 for n in history[history.index(count-1):]), history)

    def test_desktop_badge_preference_clears_and_restores_the_native_count(self):
        self.mcp.call("desktop.start", desktop_badges=True)
        self.mcp.batch(check("desktop_badge.visible",True))
        count = self.mcp.call("desktop.state")["desktop_badge"]["count"]
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),click(1150,88),type_text("badge"),
                       check("settings_matches",["Mail & performance"]),click(500,289),
                       check("settings_group","Mail & performance"), shot("badge-preference"),
                       click(340,431),check("desktop_badge.count",0),check("desktop_badge.visible",False),
                       check("preferences_saved",True),key("ctrl+1"),key("ctrl+comma"),
                       click(1150,88),type_text("badge"),click(500,289),wait(),click(340,431),
                       check("desktop_badge.count",count),check("desktop_badge.visible",True),
                       click(300,239),check("settings_group",None),click(286,737),check("unified",False),
                       key("ctrl+1"),check("account","preview-work"),check("desktop_badge.count",count),
                       shot("badge-count-spans-account-inboxes"))
        self.mcp.call("desktop.start",desktop_badges=True,width=900,height=640)
        self.mcp.batch(check("desktop_badge.visible",True),key("ctrl+comma"),check("tab","Preferences"),
                       click(563,366),check("dark",True),click(650,88),type_text("badge"),
                       check("settings_search","badge"),check("settings_matches",["Mail & performance"]),wait(80),
                       click(450,289),check("settings_group","Mail & performance"),
                       shot("badge-preference-compact-dark"),click(310,431),
                       check("desktop_badge.visible",False),check("saved_unread_badge",False))

    def test_filtered_preferences_do_not_leave_pixels_outside_scroll_view(self):
        result = self.mcp.call("desktop.start", width=900, height=640)
        directory = Path(result["artifacts"])
        print(f"Preferences clipping evidence: {directory}", flush=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"),
                       click(563,366), check("dark", True), shot("preferences-before-filter"),
                       click(650,88), type_text("badge"), check("settings_search", "badge"),
                       check("settings_matches", ["Mail & performance"]), wait(80),
                       click(450,289), check("settings_group", "Mail & performance"),
                       shot("preferences-filtered"),
                       {"type":"resize", "width":901, "height":640}, check("window_size", [901,640]),
                       {"type":"resize", "width":900, "height":640}, check("window_size", [900,640]),
                       shot("preferences-filtered-repainted"))
        for name in ("preferences-filtered", "preferences-filtered-repainted"):
            # This is the empty bottom margin, below the scroll viewport. The old
            # scale picker's cached text escaped its clip and survived filtering.
            pixels = subprocess.check_output(["convert", str(directory / f"{name}.webp"),
                                              "-crop", "80x17+780+614", "-depth", "8", "rgb:-"])
            self.assertLessEqual(max(pixels)-min(pixels), 16, f"Stray control pixels in {name}")

    def test_desktop_badge_archive_delete_move_and_undo(self):
        for action in ("archive", "delete", "move"):
            with self.subTest(action=action):
                self.mcp.call("desktop.start", desktop_badges=True, mail_actions="slow")
                self.mcp.batch(check("unread",True),check("desktop_badge.visible",True))
                count = self.mcp.call("desktop.state")["desktop_badge"]["count"]
                if action == "archive":
                    self.mcp.batch(click(652,100))
                elif action == "delete":
                    self.mcp.batch(key("ctrl+d"))
                else:
                    self.mcp.batch(key("m"),check("focused_input","folder-search"),type_text("Projects"),key("Return"),check("dialog",None))
                self.mcp.batch(check("mail_pending",1),check("desktop_badge.count",count-1),
                               click(85,398),check("folder","Archive"),
                               {**check("mail_pending",0),"timeout_ms":5000},check("desktop_badge.count",count-1),
                               click(1340,874),check("mail_pending",1),check("desktop_badge.count",count),
                               {**check("mail_pending",0),"timeout_ms":5000},check("desktop_badge.count",count),
                               shot("badge-"+action+"-undo"))

    def test_desktop_badge_background_arrival_and_failed_action(self):
        self.mcp.call("desktop.start",desktop_badges=True,background_sync=True)
        self.mcp.batch(check("desktop_badge.visible",True),check("background_sync",True))
        count = self.mcp.call("desktop.state")["desktop_badge"]["count"]
        self.mcp.batch(check("total",121),check("desktop_badge.count",count+1),shot("badge-new-arrival"))
        self.mcp.call("desktop.start",desktop_badges=True,mail_actions="fail")
        self.mcp.batch(check("desktop_badge.visible",True),check("unread",True))
        count = self.mcp.call("desktop.state")["desktop_badge"]["count"]
        self.mcp.batch(click(652,100),check("mail_pending",1),check("desktop_badge.count",count-1),
                       {**check("mail_pending",0),"timeout_ms":5000},check("notice","restored","contains"),
                       check("desktop_badge.count",count),shot("badge-failed-archive"))

    def test_background_sync_failure_allows_manual_retry(self):
        self.mcp.call("desktop.start", background_sync=True, sync_failure_once=True)
        self.mcp.batch(check("background_sync", True), check("refreshing", False),
                       check("notice", "temporarily unavailable", "contains"),
                       check("background_sync", False), check("total", 120), shot("background-sync-error"),
                       click(1400, 36), check("refreshing", True),
                       check("sync_round", 2), check("total", 121), check("refreshing", False),
                       check("notice", None),
                       click(420, 247), check("selected", "New mail from the background"),
                       shot("background-sync-retry-arrival"))

    def test_mail_check_interval_uses_seconds_and_validates_before_saving(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"),
                       click(1150, 88), type_text("background"), check("settings_matches", ["Mail & performance"]),
                       click(500, 289), check("settings_group", "Mail & performance"), shot("mail-check-seconds-setting"),
                       click(1110, 364), key("ctrl+a"), type_text("0"), click(1350, 88),
                       check("notice", "5–3600 seconds", "contains"), check("mail_check_seconds", 15),
                       click(1110, 364), key("ctrl+a"), type_text("5"), click(1350, 88),
                       check("mail_check_seconds", 5), check("preferences_saved", True), check("notice", None),
                       shot("mail-check-seconds-saved"), key("ctrl+1"), key("ctrl+comma"),
                       check("fields.mail_check_seconds", "5"))

    def test_read_unread_and_flags_show_immediately_during_slow_save(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(check("unread",None,"ne"), check("starred",None,"ne"))
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
        self.mcp.batch(check("unread",None,"ne"), check("starred",None,"ne"))
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

    def test_action_toasts_count_immediately_and_dismiss_before_saving(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(click(652,100), check("total",119), check("mail_pending",1),
                       check("action_toast.label","Archived 1 message"),
                       key("Delete"), check("total",118), check("mail_pending",2),
                       check("action_toast.label","Archived 2 messages"), shot("counted-archive-pending"),
                       click(1390,874), check("action_toast",None),
                       {**check("mail_pending",0),"timeout_ms":5000}, check("action_toast",None),
                       key("ctrl+d"), check("total",117), check("mail_pending",1),
                       check("action_toast.label","Deleted 1 message"),
                       key("ctrl+d"), check("total",116), check("mail_pending",2),
                       check("action_toast.label","Deleted 2 messages"), shot("counted-delete-pending"),
                       {**check("mail_pending",0),"timeout_ms":5000}, check("total",116))

    def test_move_toast_counts_and_failing_actions_remove_their_feedback(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        for count in (1,2):
            self.mcp.batch(key("m"), check("dialog","Move"), check("focused_input","folder-search"),
                           type_text("Projects"), key("Return"), check("dialog",None),
                           check("action_toast.label",f"Moved {count} message{'s' if count>1 else ''} to Projects"),
                           check("total",120-count))
        self.mcp.batch(check("mail_pending",2), shot("counted-move-pending"),
                       {**check("mail_pending",0),"timeout_ms":5000})
        self.mcp.call("desktop.start",mail_actions="fail")
        self.mcp.batch(key("BackSpace"), check("action_toast.label","Archived 1 message"),
                       key("ctrl+d"), check("action_toast.label","Deleted 1 message"),
                       check("total",118), check("mail_pending",2), shot("latest-action-before-failure"),
                       {**check("mail_pending",0),"timeout_ms":5000}, check("total",120),
                       check("action_toast",None), check("notice","restored","contains"), shot("action-toast-failure"))

    def test_compact_dark_action_toast_and_cross_account_slow_failure(self):
        self.mcp.call("desktop.start",mail_actions="fail")
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(80),
                       click(690,366),check("dark",True),
                       click(286,773),check("cross_account_moves",True),
                       key("ctrl+1"),check("tab","Mail"),wait(80),
                       key("m"),check("dialog","Move"),wait(80),
                       click(710,327),wait(80),click(710,403),check("fields.move_account","preview-personal"),
                       click(670,385),type_text("Archive"),key("Return"),check("dialog",None),
                       check("total",119),check("mail_pending",1),check("action_toast.label","Archived 1 message"),
                       shot("cross-account-toast-pending"),
                       {**check("mail_pending",0),"timeout_ms":5000},check("total",120),check("action_toast",None),
                       check("notice","Fixture server rejected","contains"),
                       click(1390,900),
                       {"type":"resize","width":900,"height":640},wait(150),
                       key("ctrl+d"),check("total",119),check("action_toast.label","Deleted 1 message"),
                       shot("dark-compact-toast"),
                       {**check("mail_pending",0),"timeout_ms":5000},check("action_toast",None))

    def test_grouped_archive_undo_is_immediate_while_both_moves_are_pending(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        initial = self.mcp.call("desktop.state")["mail_rows"]
        self.mcp.batch(click(652, 100), key("Delete"), check("total", 118),
                       check("action_toast.label", "Archived 2 messages"), check("action_toast.undo", True),
                       click(1340, 874), check("action_toast.label", "Restored 2 messages"),
                       check("action_toast.undo", False), check("total", 120),
                       check("mail_pending", 1, "gte"), shot("undo-group-before-provider-ack"),
                       key("ctrl+2"), check("tab", "Calendar"),
                       {**check("mail_pending", 1, "lte"), "timeout_ms": 5000},
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       key("ctrl+1"), check("tab", "Mail"), check("total", 120), wait(150),
                       shot("undo-group-restored-inbox"))
        restored = self.mcp.call("desktop.state")["mail_rows"]
        self.assertEqual([m["subject"] for m in restored[:2]], [m["subject"] for m in initial[:2]])
        self.mcp.batch(click(85, 398), check("folder", "Archive"), check("total", 0))

    def test_delete_undo_failure_has_persistent_retry_and_restores_after_retry(self):
        self.mcp.call("desktop.start", mail_actions="slow", undo_failure_once=True)
        subject = self.selected_mail_subject()
        self.mcp.batch(key("ctrl+d"), check("action_toast.label", "Deleted 1 message"),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, check("total", 119),
                       click(1340, 874), check("total", 120), check("action_toast.label", "Restored 1 message"),
                       check("mail_pending", 1), shot("undo-delete-pending"),
                       {**check("undo_failures", 1), "timeout_ms": 5000}, check("total", 119),
                       check("mail_pending", 0), check("notice", "Fixture server rejected Undo", "contains"),
                       shot("undo-failed-retry-control"), click(1308, 874),
                       check("undo_failures", 0), check("total", 120), check("mail_pending", 1),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, check("total", 120),
                       check("notice", None), shot("undo-delete-retried"))
        self.assertIn(subject, [m["subject"] for m in self.mcp.call("desktop.state")["mail_rows"]])
        self.mcp.batch(click(85, 438), check("folder", "Trash"), check("total", 0))

    def test_move_undo_from_destination_and_compact_dark_feedback(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        subject = self.selected_mail_subject()
        self.mcp.batch(key("m"), check("dialog", "Move"), check("focused_input", "folder-search"),
                       type_text("Projects"), key("Return"), check("dialog", None),
                       check("action_toast.label", "Moved 1 message to Projects"),
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       click(95, 537), check("folder", "Projects"), check("total", 1),
                       click(1340, 874), check("total", 0), check("mail_pending", 1),
                       check("action_toast.label", "Restored 1 message"),
                       {**check("mail_pending", 0), "timeout_ms": 5000}, check("total", 0), check("notice", None),
                       click(85, 115), check("folder", "INBOX"), check("total", 120),
                       key("ctrl+comma"), check("tab", "Preferences"), wait(80), click(690, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), wait(80),
                       {"type": "resize", "width": 900, "height": 640}, wait(150),
                       key("ctrl+d"), check("action_toast.label", "Deleted 1 message"),
                       shot("undo-dark-compact-control"), click(800, 594),
                       check("action_toast.label", "Restored 1 message"), check("total", 120),
                       shot("undo-dark-compact-restoring"),
                       {**check("mail_pending", 0), "timeout_ms": 7000}, check("total", 120))
        self.assertIn(subject, [m["subject"] for m in self.mcp.call("desktop.state")["mail_rows"]])

    def test_cross_account_undo_returns_to_original_account_while_settings_remain_usable(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), wait(80),
                       click(286, 773), check("cross_account_moves", True),
                       key("ctrl+1"), check("tab", "Mail"), wait(80),
                       key("m"), check("dialog", "Move"), wait(80),
                       click(710, 327), wait(80), click(710, 403), check("fields.move_account", "preview-personal"),
                       click(670, 385), type_text("Archive"), key("Return"), check("dialog", None),
                       check("total", 119), check("action_toast.label", "Archived 1 message"),
                       click(1340, 874), check("total", 120), check("mail_pending", 1),
                       key("ctrl+comma"), check("tab", "Preferences"), wait(80),
                       click(286, 773), check("cross_account_moves", False),
                       {**check("mail_pending", 0), "timeout_ms": 5000},
                       key("ctrl+1"), check("tab", "Mail"), check("total", 120),
                       check("mail_rows.0.account_id", "preview-work"), check("notice", None), shot("undo-cross-account-restored"),
                       click(85, 398), check("folder", "Archive"), check("total", 0))

    def test_read_on_leave_updates_immediately_and_preserves_explicit_unread(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(key("ctrl+2"), check("tab", "Calendar"), key("ctrl+1"), check("tab", "Mail"), wait(80),
                       check("mail_rows.0.unread", True), check("mail_pending", 0),
                       click(420,246), check("selected", "A little more room to think"), check("unread", True), check("read_candidate", "A little more room to think"),
                       click(420,345), check("selected", "Your weekly workspace digest"),
                       check("mail_rows.0.unread", False), check("mail_rows.1.unread", True),
                       check("mail_pending", 1), shot("read-on-leave-before-save"),
                       {**check("mail_pending", 0), "timeout_ms":5000},
                       click(420,246), check("selected", "A little more room to think"),
                       {**check("mail_pending", 0), "timeout_ms":5000},
                       click(740,100), check("unread", True), check("mail_pending",1),
                       click(420,345), check("selected", "Your weekly workspace digest"),
                       {**check("mail_pending", 0), "timeout_ms":5000}, check("mail_rows.0.unread",True),
                       key("ctrl+r"), check("refreshing", True), check("refreshing", False),
                       check("mail_rows.0.unread",True), shot("explicit-unread-survives-leaving"))

    def test_read_on_leave_failure_preserves_folder_and_unread(self):
        self.mcp.call("desktop.start", mail_actions="fail")
        self.mcp.batch(click(420,246), check("selected", "A little more room to think"),
                       click(84,398), check("folder", "Archive"), check("mail_pending",1),
                       {**check("mail_pending",0),"timeout_ms":5000},
                       check("notice","Could not update this message","contains"), check("folder","Archive"),
                       shot("read-on-leave-failure"), click(84,278), check("folder","INBOX"),
                       check("mail_rows.0.unread",True), check("mail_pending",0))

    def test_read_on_leave_with_arrows_in_unread_filter_keeps_next_selection(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        self.mcp.batch(click(350,100), wait(100), click(350,155), check("filter","Unread"), check("total",4),
                       click(420,246), check("selected","A little more room to think"), key("Down"),
                       check("selected","Your weekly workspace digest"), check("total",3),
                       check("mail_rows.0.subject","Your weekly workspace digest"), check("mail_pending",1),
                       key("Down"), check("selected","Coffee next Thursday?"), check("total",2),
                       {**check("mail_pending",0),"timeout_ms":5000},
                       check("selected","Coffee next Thursday?"), check("total",2), shot("read-on-leave-unread-navigation"))

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
        # Ready precedes the initial body/layout presentation. Begin the native
        # drag after that presentation, as in the other panel-resize flows.
        self.mcp.batch(check("reader_text_ready", True), wait(150),
                       drag(222, 500, 310, 500), check("sidebar_width", 305, "gte"),
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
                       drag(701, 333, 765, 333), check("reader_selected_text", "Hey Alex", "contains"),
                       key("ctrl+c"), shot("selected-email-text"),
                       key("ctrl+k"), check("focused_input", "search"), key("ctrl+v"), check("query", "Hey Alex", "contains"),
                       key("ctrl+a"), key("BackSpace"), check("total", 120), key("Escape"),
                       double_click(420, 243), check("full_reader", True), check("reader_text_ready", True),
                       click(500, 410), key("ctrl+a"), check("reader_selected_text", "Design lead", "contains"),
                       key("ctrl+c"), shot("full-reader-selectable"), key("m"), check("dialog", "Move"),
                       key("Escape"), check("dialog", None), key("Escape"), check("full_reader", False),
                       key("ctrl+comma"), check("tab", "Preferences"), click(690, 366), check("dark", True),
                       key("ctrl+1"), check("tab", "Mail"), wait(80), drag(701, 333, 765, 333),
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

    def test_bulk_archive_review_cancellation_immediate_undo_and_delete_review(self):
        result = self.mcp.call("desktop.start", mail_actions="slow")
        print(f"Bulk review and Undo evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(click(584,164), check("mail_selection.mode",True),
                       key("ctrl+a"), check("mail_selection.count",120),
                       check("mail_selection.pending",False),
                       key("BackSpace"), check("dialog","BulkReview"),
                       check("bulk.review_count",120), check("bulk.action","Archive"),
                       shot("bulk-archive-review"), key("n"), check("dialog",None),
                       check("total",120), check("mail_selection.count",120),
                       key("ctrl+d"), check("dialog","BulkReview"),
                       check("bulk.action","Move to Trash"), shot("bulk-delete-review"),
                       key("Escape"), check("dialog",None), check("total",120),
                       key("BackSpace"), check("dialog","BulkReview"),
                       check("bulk.review_count",120), key("y"), check("dialog",None),
                       check("total",0), check("action_toast.count",120),
                       shot("bulk-archive-immediate"), check("bulk.jobs.0.running",1), click(1340,874),
                       check("action_toast.label","Restored 120 messages"), check("total",120),
                       shot("bulk-undo-immediate"), {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.failed",0), check("bulk.jobs.0.uncertain",0),
                       check("total",120), shot("bulk-undo-complete"))

    def test_bulk_flags_read_and_move_use_the_selected_messages(self):
        self.mcp.batch(click(584,164), check("mail_selection.mode",True), check("mail_selection.drawn",True), click(274,218), click(274,322),
                       check("mail_selection.count",2), check("mail_selection.pending",False),
                       key("s"), check("dialog","BulkReview"), check("bulk.action","Flag"),
                       check("bulk.review_count",2), key("Return"), check("dialog",None),
                       check("bulk.jobs.0.remaining",0), check("mail_rows.0.starred",True),
                       check("mail_rows.1.starred",True), shot("bulk-flagged"),
                       click(584,164), check("mail_selection.mode",True), check("mail_selection.drawn",True), click(274,218), click(274,322),
                       check("mail_selection.pending",False), click(732,100),
                       check("dialog","BulkReview"), check("bulk.action","Mark as read"),
                       key("Return"), check("dialog",None), check("bulk.jobs.0.remaining",0),
                       check("mail_rows.0.unread",False), check("mail_rows.1.unread",False),
                       check("inbox_unread.preview-work",1), check("inbox_unread.preview-personal",1), shot("bulk-read"),
                       click(584,164), check("mail_selection.mode",True), check("mail_selection.drawn",True), click(274,218), click(274,322),
                       check("mail_selection.pending",False), click(732,100),
                       check("dialog","BulkReview"), check("bulk.action","Mark as unread"),
                       key("Return"), check("dialog",None), check("bulk.jobs.0.remaining",0),
                       check("inbox_unread.preview-work",3), click(584,164), check("mail_selection.mode",True), check("mail_selection.drawn",True), click(274,218), click(274,426),
                       check("mail_selection.count",2), check("mail_selection.pending",False), key("m"), check("dialog","Move"),
                       check("focused_input","folder-search"), type_text("Projects"),
                       key("Return"), check("dialog","BulkReview"), check("bulk.review_count",2),
                       shot("bulk-move-review"), key("Return"), check("dialog",None),
                       check("total",118), check("action_toast.label","Moved 2 messages to Projects"),
                       check("bulk.jobs.0.remaining",0), click(80,535), check("folder","Projects"),
                       check("total",1), shot("bulk-move-destination"), click(1340,874),
                       check("action_toast.label","Restored 2 messages"),
                       check("bulk.jobs.0.remaining",0), check("total",0),
                       click(80,278), check("folder","INBOX"), check("total",120))

    def test_bulk_failures_restore_rows_and_keep_reviewable_results(self):
        result=self.mcp.call("desktop.start",mail_actions="fail")
        print(f"Bulk failure evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(click(584,164), check("mail_selection.mode",True), check("mail_selection.drawn",True),
                       click(274,218), click(274,322),
                       check("mail_selection.pending",False), key("ctrl+d"),
                       check("dialog","BulkReview"), check("bulk.review_count",2),
                       key("Return"), check("dialog",None), check("total",118),
                       check("action_toast.count",2), check("bulk.jobs.0.running",1),
                       shot("bulk-delete-pending"), {**check("bulk.jobs.0.failed",2),"timeout_ms":5000},
                       check("bulk.jobs.0.remaining",0), check("total",120),
                       check("notice","2 messages could not be confirmed","contains"),
                       shot("bulk-delete-failed"), click(1330,36), check("dialog","BulkHistory"),
                       wait(100), shot("bulk-failure-history"), click(700,490),
                       check("bulk.items.0.status","failed"), check("bulk.items.1.status","failed"),
                       wait(100), shot("bulk-failure-items"), {"type":"resize","width":900,"height":640},
                       wait(150), shot("bulk-failure-items-compact"))

    def test_bulk_reviews_and_pending_undo_in_compact_dark(self):
        result=self.mcp.call("desktop.start",mail_actions="slow")
        print(f"Compact bulk evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(100),
                       click(690,366),check("dark",True),key("ctrl+1"),check("tab","Mail"),wait(100),
                       click(584,164),check("mail_selection.mode",True),check("mail_selection.drawn",True),
                       click(274,218),click(274,322),check("mail_selection.count",2),
                       check("mail_selection.pending",False),
                       {"type":"resize","width":900,"height":640},wait(150),
                       key("ctrl+d"),check("dialog","BulkReview"),check("bulk.review_count",2),
                       shot("bulk-dark-delete-review"),key("n"),check("dialog",None),check("total",120),
                       key("Delete"),check("dialog","BulkReview"),check("bulk.action","Archive"),
                       key("Return"),check("dialog",None),check("total",118),
                       check("action_toast.count",2),check("bulk.jobs.0.running",1),
                       shot("bulk-dark-archive-pending"),click(800,594),
                       check("action_toast.label","Restored 2 messages"),check("total",120),
                       shot("bulk-dark-undo-immediate"),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.failed",0),check("total",120),shot("bulk-dark-undo-complete"))

    def test_bulk_pending_rows_reject_conflicting_context_actions_and_enable_after_completion(self):
        result=self.mcp.call("desktop.start", mail_actions="slow")
        print(f"Bulk conflict evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(click(584,164),check("mail_selection.mode",True),check("mail_selection.drawn",True),
                       click(274,322),click(274,426),check("mail_selection.count",2),
                       check("mail_selection.pending",False),key("s"),check("dialog","BulkReview"),
                       key("Return"),check("dialog",None),check("bulk.jobs.0.running",1),
                       check("mail_rows.1.group_pending",True),check("selected","A little more room to think"),
                       click(568,322),check("mail_pending",0),check("selected","A little more room to think"),
                       {"type":"click","x":400,"y":350,"button":3},check("context_menu",None,"ne"),
                       key("Down"),key("Down"),key("Down"),key("Return"),check("context_menu",None),
                       check("notice","part of a group change","contains"),check("mail_pending",0),
                       shot("bulk-row-conflict-keeps-group"),
                       {**check("bulk.jobs.0.remaining",0),"timeout_ms":5000},
                       check("bulk.jobs.0.failed",0),check("mail_rows.1.group_pending",False),wait(80),
                       click(568,322),check("mail_rows.1.starred",False),check("mail_pending",1),
                       check("mail_pending",0),shot("bulk-row-controls-restored"))

    def test_selection_mode_row_clicks_toggle_without_clearing_other_pages(self):
        self.mcp.batch(check("selected", "A little more room to think"),
                       click(574,155), check("mail_selection.mode",True),
                       check("mail_selection.drawn",True),
                       click(400,245), click(400,453),
                       check("mail_selection.count",2), check("mail_selection.pending",False),
                       click(400,245), check("mail_selection.count",1),
                       click(400,349), check("mail_selection.count",2),
                       {"type":"click","x":400,"y":245,"modifiers":["shift"]},
                       check("mail_selection.count",3), check("mail_selection.pending",False),
                       check("selected", "A little more room to think"),
                       shot("selection-additive-row-range"),
                       click(586,884), check("offset",50), check("mail_selection.pending",False),
                       click(400,245), check("mail_selection.count",4),
                       click(400,453), check("mail_selection.count",5),
                       click(400,245), check("mail_selection.count",4),
                       check("mail_selection.pending",False),
                       key("BackSpace"), check("dialog","BulkReview"),
                       check("bulk.review_count",4), shot("selection-additive-cross-page-review"),
                       key("Escape"), check("dialog",None),
                       key("Escape"), check("mail_selection.mode",False),
                       click(400,349), check("mail_selection.mode",False))

    def test_mail_selection_mouse_ranges_and_focus(self):
        self.mcp.batch(click(400, 255), check("selected", "A little more room to think"),
                       {"type":"click", "x":400, "y":360, "modifiers":["ctrl"]},
                       check("mail_selection.count", 2), check("mail_selection.pending", False),
                       {"type":"click", "x":400, "y":568, "modifiers":["shift"]},
                       check("mail_selection.count", 4), check("mail_selection.pending", False),
                       shot("selection-range"),
                       {"type":"click", "x":400, "y":450, "modifiers":["ctrl"]},
                       check("mail_selection.count", 3),
                       {"type":"click", "x":400, "y":450, "modifiers":["ctrl"]},
                       check("mail_selection.count", 4), check("full_reader", False),
                       double_click(400, 250), check("full_reader", True),
                       key("Escape"), check("full_reader", False), key("ctrl+a"),
                       check("mail_selection.count", 120), check("mail_selection.pending", False),
                       key("Escape"), check("mail_selection.mode", False),
                       key("ctrl+k"), check("focused_input", "search"), type_text("Coffee next Thursday"),
                       check("total", 1), key("ctrl+a"), type_text("prototype"), check("total", 1),
                       check("mail_selection.mode", False), shot("selection-text-focus"))

    def test_mail_selection_checkbox_can_include_an_arrival_without_losing_prior_choices(self):
        result=self.mcp.call("desktop.start", background_sync=True)
        print(f"Arrival checkbox evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("background_sync", True), click(584,164),
                       check("mail_selection.mode",True), check("mail_selection.drawn",True),
                       click(274,218), check("mail_selection.count",1), check("mail_selection.pending",False),
                       check("total",121), check("mail_rows.0.subject","New mail from the background"),
                       check("mail_selection.count",1), check("mail_selection.pending",False),
                       shot("selection-arrival-unselected"), click(274,218),
                       check("mail_selection.count",2), check("mail_selection.pending",False),
                       check("mail_selection.available",2), shot("selection-arrival-checkbox"),
                       key("BackSpace"), check("dialog","BulkReview"), check("bulk.review_count",2),
                       key("Escape"), check("dialog",None), check("mail_selection.count",2))

    def test_mail_selection_ctrl_click_and_shift_range_follow_arrivals(self):
        result=self.mcp.call("desktop.start", background_sync=True)
        print(f"Arrival range evidence: {result['artifacts']}", flush=True)
        self.mcp.batch(check("background_sync",True), click(584,164),
                       check("mail_selection.mode",True), check("mail_selection.drawn",True),
                       click(274,218), check("mail_selection.count",1), check("mail_selection.pending",False),
                       check("total",121), check("mail_rows.0.subject","New mail from the background"),
                       check("mail_selection.count",1),
                       {"type":"click","x":400,"y":250,"modifiers":["ctrl"]},
                       check("mail_selection.count",2), check("mail_selection.pending",False),
                       shot("selection-arrival-ctrl"),
                       {"type":"click","x":400,"y":460,"modifiers":["shift"]},
                       check("mail_selection.count",3), check("mail_selection.pending",False),
                       check("mail_selection.available",3), shot("selection-arrival-range"),
                       key("m"), check("dialog","Move"), key("Escape"), check("dialog",None),
                       check("mail_selection.count",3))

    def test_mail_selection_checkboxes_pages_scope_and_compact(self):
        self.mcp.batch(click(584, 164), check("mail_selection.mode", True), check("mail_selection.drawn", True),
                       click(274, 218), check("mail_selection.count", 1),
                       click(274, 322), check("mail_selection.count", 2),
                       check("mail_selection.pending", False),
                       click(260, 218), check("mail_selection.count", 1),
                       click(274, 218), check("mail_selection.count", 2),
                       check("selected", "A little more room to think"), shot("selection-checkboxes"),
                       key("ctrl+a"), check("mail_selection.count", 120),
                       check("mail_selection.pending", False), click(586, 884), check("offset", 50),
                       check("mail_selection.pending", False), shot("selection-next-page"),
                       {"type":"resize", "width":900, "height":640}, wait(150),
                       shot("selection-compact"), click(80, 398), check("folder", "Archive"),
                       check("mail_selection.mode", False))

    def test_mail_selection_remap_reader_isolation_and_dark(self):
        self.mcp.batch(click(950, 325), check("mail_selection.list_focus", False), key("ctrl+a"), wait(80), check("mail_selection.mode", False),
                       click(400, 250), key("Tab"), check("sidebar_focus", True),
                       key("ctrl+a"), wait(80), check("mail_selection.mode", False),
                       key("Tab"), check("sidebar_focus", False), key("ctrl+a"),
                       check("mail_selection.count", 120), check("mail_selection.pending", False),
                       click(571, 104), check("mail_selection.count", 0),
                       check("mail_selection.mode", True), key("Escape"), check("mail_selection.mode", False),
                       key("ctrl+comma"), check("tab", "Preferences"), wait(80),
                       click(690, 366), check("dark", True), click(645, 156), check("settings_tab", "Shortcuts"),
                       {"type":"hover", "x":1200, "y":700}, {"type":"scroll", "amount":30}, wait(150),
                       click(920, 480), key("alt+a"), check("shortcuts.SelectAll", "Alt+A"),
                       check("preferences_saved", True), shot("selection-shortcut-remapped"),
                       key("ctrl+1"), check("tab", "Mail"), wait(80), click(400, 250),
                       key("ctrl+a"), check("mail_selection.mode", False), key("alt+a"),
                       check("mail_selection.count", 120), check("mail_selection.pending", False),
                       shot("selection-dark"), {"type":"resize", "width":900, "height":640},
                       wait(150), shot("selection-compact-dark"))

    def test_read_search_preload_and_mouse_navigation(self):
        self.mcp.batch(check("selected", "A little more room to think"), check("cache_entries", 3, "gte"),
                       check("page_prefetched", True), shot("mail-light"),
                       click(403, 450), check("selected", "Coffee next Thursday?"),
                       key("ctrl+k"), check("focused_input", "search"), type_text("prototype"), check("total", 1),
                       check("selected", "Re: A few thoughts on the prototype"), shot("search-results"),
                       key("ctrl+a"), type_text("no-match-938481"), check("total", 0),
                       key("ctrl+a"), key("BackSpace"), check("total", 120), key("Escape"))

    def test_search_finds_other_folders_moves_results_and_returns_to_browsing_folder(self):
        started=self.mcp.call("desktop.start",long_folders=True,mail_actions="slow")
        print(f"Across-folder search evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(check("folder","INBOX"),check("total",120),key("ctrl+k"),check("focused_input","search"),
                       type_text("Sidebar fixture"),check("total",4),key("Escape"),check("reader_text_ready",True),shot("search-other-folders"))
        state=self.mcp.call("desktop.state")
        self.assertTrue(all(row["folder"]!="INBOX" for row in state["mail_rows"]))
        subject=state["selected"]
        self.mcp.batch(key("m"),check("focused_input","folder-search"),type_text("Projects"),check("move_enter_destination","Projects"),key("Return"),
                       check("dialog",None),check("mail_pending",1),check("total",4),check("selected",subject),
                       shot("search-result-move-pending"),check("mail_pending",0),check("reader_text_ready",True))
        state=self.mcp.call("desktop.state")
        moved=next(row for row in state["mail_rows"] if row["subject"]==subject)
        self.assertEqual(moved["folder"],"Projects")
        self.mcp.batch(key("ctrl+k"),check("focused_input","search"),key("ctrl+a"),key("BackSpace"),
                       check("query",""),check("folder","INBOX"),check("total",120),key("Escape"),
                       click(85,536),check("folder","Projects"),check("total",2),shot("search-cleared-browsing-restored"))
        self.assertIn(subject,[row["subject"] for row in self.mcp.call("desktop.state")["mail_rows"]])

    def test_search_bulk_selection_includes_matches_in_all_result_folders(self):
        started=self.mcp.call("desktop.start",long_folders=True)
        print(f"Across-folder selection evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("Sidebar fixture"),check("total",4),key("Escape"),
                       click(400,245),key("ctrl+a"),check("mail_selection.count",4),check("mail_selection.pending",False),
                       key("Delete"),check("dialog","BulkReview"),check("bulk.review_count",4),shot("search-all-folders-bulk-review"),
                       key("Return"),check("dialog",None),check("bulk.jobs.0.completed",4),check("total",4))
        self.assertTrue(all(row["folder"]=="Archive" for row in self.mcp.call("desktop.state")["mail_rows"]))
        self.mcp.batch(key("Escape"),key("ctrl+k"),check("focused_input","search"),key("ctrl+a"),key("BackSpace"),
                       check("query",""),check("total",120),key("Escape"),click(85,399),check("folder","Archive"),check("total",4),shot("search-bulk-archive-verified"))

    def test_search_respects_account_scope_and_shows_folder_labels_in_compact_dark_layout(self):
        started=self.mcp.call("desktop.start",long_folders=True)
        print(f"Search account scope evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("Coffee next Thursday"),check("total",1),
                       key("Escape"),check("mail_rows.0.account_id","preview-personal"),
                       key("ctrl+comma"),check("tab","Preferences"),wait(100),click(690,366),check("dark",True),
                       click(286,737),check("unified",False),key("ctrl+1"),check("tab","Mail"),
                       key("ctrl+k"),check("focused_input","search"),type_text("Coffee next Thursday"),check("total",0),
                       key("ctrl+a"),type_text("Sidebar fixture"),check("total",4),key("Escape"),
                       {"type":"resize","width":900,"height":640},wait(120),shot("search-folders-dark-compact"))
        self.assertTrue(all(row["account_id"]=="preview-work" for row in self.mcp.call("desktop.state")["mail_rows"]))

    def test_move_recovery_review_preserves_original_and_requires_explicit_choice(self):
        for mode in ("committed", "copied", "unconfirmed"):
            result=self.mcp.call("desktop.start",move_recovery=mode,persistent=True)
            print(f"Move review {mode}: {result['artifacts']}",flush=True)
            if mode=="committed":
                self.mcp.batch(click(85,536),check("folder","Projects"))
            else:
                self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("keepsake"),check("total",1),key("Escape"),check("focused_input",None))
            self.mcp.batch(check("selected","Recovered keepsake"),check("reader_text_ready",True),
                           click(1340,192),check("dialog","MoveRecovery"),
                           check("move_recovery.stage",{"committed":"Committed","copied":"Copied","unconfirmed":"Started"}[mode]),
                           wait(100),shot("move-review-"+mode))
            if mode=="unconfirmed":
                self.mcp.batch(key("Return"),key("y"),wait(100),check("move_recovery.pending",0),
                               check("dialog","MoveRecovery"),check("move_recovery.confirmed",False))
            self.mcp.batch(key("Escape"),check("dialog",None),check("selected","Recovered keepsake"),
                           check("reader_text_ready",True),check("move_recovery.total",1))

    def test_move_recovery_finishes_verified_copy_and_keeps_navigation_available(self):
        for mode in ("copied", "unconfirmed"):
            result=self.mcp.call("desktop.start",move_recovery=mode,persistent=True)
            print(f"Move recovery completion {mode}: {result['artifacts']}",flush=True)
            self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("keepsake"),check("total",1),key("Escape"),
                           check("selected","Recovered keepsake"),check("reader_text_ready",True),click(1340,192),check("dialog","MoveRecovery"),wait(100))
            if mode=="unconfirmed":
                self.mcp.batch(click(470,571),check("move_recovery.confirmed",True))
            self.mcp.batch(key("y"),check("move_recovery.pending",1),shot("move-recovery-running"),
                           key("Escape"),check("dialog",None),key("ctrl+2"),check("tab","Calendar"),
                           check("move_recovery.pending",0),check("notice","Move recovered.","contains"),
                           key("ctrl+1"),check("tab","Mail"),click(85,636 if mode=="copied" else 536),
                           check("folder","Projects"),check("selected","Recovered keepsake"),check("reader_text_ready",True),
                           check("mail_rows.0.group_pending",False),check("move_recovery.total",0),
                           shot("move-recovery-finished"),{"type":"restart"},
                           click(85,636 if mode=="copied" else 536),check("selected","Recovered keepsake"),
                           check("reader_text_ready",True),check("mail_rows.0.group_pending",False),shot("move-recovery-finished-restarted"))

    def test_move_recovery_failure_keeps_cached_reader_and_can_retry(self):
        result=self.mcp.call("desktop.start",move_recovery="fail-once",persistent=True)
        print(f"Move recovery failure evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(click(85,536),check("selected","Recovered keepsake"),check("reader_text_ready",True),
                       click(1340,192),check("dialog","MoveRecovery"),wait(100),click(920,588),
                       check("move_recovery.pending",1),key("Escape"),check("dialog",None),
                       key("ctrl+2"),check("tab","Calendar"),check("move_recovery.pending",0),
                       check("notice","temporarily unavailable","contains"),key("ctrl+1"),check("tab","Mail"),
                       click(85,536),check("selected","Recovered keepsake"),check("reader_text_ready",True),
                       check("mail_rows.0.group_pending",True),click(1340,192),check("dialog","MoveRecovery"),
                       wait(100),shot("move-recovery-retry-error"),key("Return"),check("move_recovery.pending",1),
                       check("move_recovery.pending",0),check("dialog",None),check("move_recovery.total",0),
                       check("selected","Recovered keepsake"),check("mail_rows.0.group_pending",False),
                       check("notice","Move recovered.","contains"),shot("move-recovery-retry-complete"))

    def test_move_recovery_local_copy_has_confirmation_and_survives_sync_restart(self):
        result=self.mcp.call("desktop.start",move_recovery="unconfirmed",persistent=True)
        print(f"Move recovery local evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("keepsake"),check("total",1),key("Escape"),
                       check("selected","Recovered keepsake"),check("reader_text_ready",True),
                       click(1340,192),check("dialog","MoveRecovery"),wait(100),click(720,485),
                       check("move_recovery.action","KeepLocal"),check("move_recovery.confirmed",False),
                       key("Return"),wait(80),check("move_recovery.pending",0),shot("move-recovery-keep-local-review"),
                       click(470,560),check("move_recovery.confirmed",True),key("Return"),check("dialog",None),
                       check("move_recovery.total",0),check("notice","Local copy kept","contains"),
                       check("selected_id","local-recovered-","contains"),check("reader_text_ready",True),
                       check("mail_rows.0.group_pending",False),shot("move-recovery-local-copy"),
                       click(568,218),check("mail_rows.0.starred",True),check("mail_pending",0),
                       click(1400,36),check("refreshing",False),check("selected_id","local-recovered-","contains"),
                       {"type":"restart"},key("ctrl+k"),check("focused_input","search"),type_text("keepsake"),check("total",1),key("Escape"),
                       check("selected_id","local-recovered-","contains"),check("reader_text_ready",True),
                       check("mail_rows.0.starred",True),shot("move-recovery-local-after-restart"))

    def test_move_recovery_preferences_entry_and_compact_dark_local_review(self):
        result=self.mcp.call("desktop.start",move_recovery="unconfirmed",persistent=True)
        print(f"Move recovery compact Preferences evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(100),click(690,366),check("dark",True),
                       click(383,156),check("settings_tab","Accounts"),shot("move-recovery-accounts-entry"),
                       click(380,530),check("dialog","MoveRecovery"),check("move_recovery.stage","Started"),
                       {"type":"resize","width":900,"height":640},wait(120),shot("move-recovery-review-dark-compact"),
                       click(450,345),check("move_recovery.action","KeepLocal"),wait(80),shot("move-recovery-local-dark-compact"),
                       key("Return"),check("move_recovery.pending",0),click(200,420),check("move_recovery.confirmed",True),
                       key("y"),check("dialog",None),check("move_recovery.total",0),
                       check("notice","Local copy kept","contains"),shot("move-recovery-accounts-completed"),
                       key("ctrl+1"),check("tab","Mail"),key("ctrl+k"),check("focused_input","search"),
                       type_text("keepsake"),check("total",1),key("Escape"),check("selected_id","local-recovered-","contains"),
                       check("reader_text_ready",True),shot("move-recovery-local-reader-dark-compact"))

    def test_move_recovery_graceful_close_observes_receipt_before_exit(self):
        started=self.mcp.call("desktop.start",move_recovery="copied",persistent=True)
        print(f"Move recovery close evidence: {started['artifacts']}",flush=True)
        self.mcp.batch(key("ctrl+k"),check("focused_input","search"),type_text("keepsake"),check("total",1),key("Escape"),
                       check("reader_text_ready",True),click(1340,192),check("dialog","MoveRecovery"),wait(80),
                       key("y"),check("move_recovery.pending",1))
        closed=self.mcp.call("desktop.close")
        self.assertEqual(closed["returncode"],0)
        database=Path(started["artifacts"])/"fixture.sqlite"
        with sqlite3.connect(database.as_uri()+"?mode=ro",uri=True) as cache:
            rows=cache.execute("SELECT stage,cache_id,data FROM mail_moves").fetchall()
            self.assertEqual(len(rows),1)
            stage,cache_id,data=rows[0]
            self.assertEqual(stage,"located")
            self.assertIsNone(cache_id)
            receipt=json.loads(data)["receipt"]
            self.assertEqual(receipt["current"]["remote_id"],"91.701")
            self.assertEqual(cache.execute("SELECT COUNT(*) FROM messages WHERE id=?",(receipt["current"]["id"],)).fetchone()[0],1)
        self.mcp.call("desktop.restart")
        self.mcp.batch(click(85,636),check("folder","Projects"),check("selected","Recovered keepsake"),
                       check("reader_text_ready",True),check("move_recovery.total",0),shot("move-recovery-after-graceful-close"))

    def test_moved_cache_is_readable_after_restart_and_refresh_rekeys_the_open_reader(self):
        result=self.mcp.call("desktop.start",move_recovery=True,persistent=True)
        print(f"Move recovery native evidence: {result['artifacts']}",flush=True)
        self.mcp.batch(click(85,536),check("folder","Projects"),check("total",1),
                       check("selected","Recovered keepsake"),check("reader_text_ready",True),
                       check("mail_rows.0.group_pending",True),shot("move-recovery-cold-cache"))
        original=self.mcp.call("desktop.state")["selected_id"]
        self.mcp.batch({"type":"restart"},click(85,536),check("folder","Projects"),
                       check("total",1),check("selected","Recovered keepsake"),
                       check("reader_text_ready",True),check("selected_id",original),
                       check("mail_rows.0.group_pending",True),shot("move-recovery-after-restart"),
                       click(1400,36),check("refreshing",True),check("selected","Recovered keepsake"),
                       check("refreshing",False),check("selected_id",original,"ne"),
                       check("selected","Recovered keepsake"),check("reader_text_ready",True),
                       check("total",1),check("mail_rows.0.group_pending",False),check("notice",None),shot("move-recovery-located"))

    def test_move_shows_destination_before_server_acknowledgment(self):
        self.mcp.call("desktop.start", mail_actions="slow")
        subject = self.selected_mail_subject()
        source_id = self.mcp.call("desktop.state")["selected_id"]
        self.mcp.batch(key("m"), check("focused_input", "folder-search"), type_text("Projects"),
                       key("Return"), check("dialog", None), check("total", 119),
                       click(85, 536), check("folder", "Projects"), check("total", 1),
                       {"type": "assert", "path": "mail_pending", "value": 1, "op": "gte"},
                       check("mail_rows.0.subject", subject), check("selected", subject),
                       check("mail_rows.0.group_pending", True), shot("move-visible-while-pending"),
                       check("mail_pending", 0), check("total", 1), check("mail_rows.0.subject", subject),
                       check("mail_rows.0.group_pending", False), check("selected_id", source_id, "ne"),
                       shot("move-visible-after-acknowledgment"),
                       key("m"), check("focused_input", "folder-search"), type_text("Inbox"), key("Return"),
                       check("dialog", None), check("total", 0), click(85, 115), check("folder", "INBOX"),
                       check("total", 120), check("mail_pending", 0), check("total", 120))

    def test_move_destination_failure_and_pending_undo_restore_source(self):
        for outcome in ("fail", "undo"):
            result = self.mcp.call("desktop.start", mail_actions="fail" if outcome == "fail" else "slow")
            print(f"Destination {outcome}: {result['artifacts']}", flush=True)
            self.mcp.batch(key("m"), check("focused_input", "folder-search"), type_text("Projects"),
                           key("Return"), check("dialog", None), check("total", 119),
                           click(85, 536), check("folder", "Projects"), check("total", 1),
                           {"type":"assert", "path":"mail_pending", "value":1, "op":"gte"},
                           check("mail_rows.0.group_pending", True))
            if outcome == "undo":
                self.mcp.batch(click(1340, 874), check("total", 0),
                               check("action_toast.label", "Restored 1 message"),
                               shot("pending-destination-undone"),
                               {**check("mail_pending", 0), "timeout_ms":5000})
            else:
                self.mcp.batch(check("mail_pending", 0), check("total", 0),
                               check("notice", "restored", "contains"), shot("destination-failure"))
            self.mcp.batch(check("total", 0), click(85, 115), check("folder", "INBOX"), check("total", 120))

    def test_cross_account_destination_is_visible_during_transfer(self):
        result = self.mcp.call("desktop.start", mail_actions="slow")
        print(f"Cross-account destination: {result['artifacts']}", flush=True)
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), wait(80), click(286, 773),
                       check("cross_account_moves", True), key("ctrl+1"), check("tab", "Mail"), wait(80))
        self.hold_mail_over(402, 245, 95, 636)
        self.mcp.batch(check("mail_drag.valid", True), {"type":"mouse_up"}, check("total", 119),
                       click(95, 636), check("folder", "Projects"), check("total", 1),
                       {"type":"assert", "path":"mail_pending", "value":1, "op":"gte"},
                       check("mail_rows.0.account_id", "preview-personal"),
                       check("mail_rows.0.group_pending", True), shot("cross-account-pending-destination"),
                       check("mail_pending", 0), check("total", 1), check("mail_rows.0.group_pending", False),
                       check("mail_rows.0.account_id", "preview-personal"), shot("cross-account-confirmed-destination"))

    def test_move_mouse_and_keyboard_and_typing_protection(self):
        self.mcp.batch(key("m"), check("dialog", "Move"), shot("move-dialog"), key("Escape"), check("dialog", None),
                       key("ctrl+k"), check("focused_input", "search"), type_text("m"), check("query", "m"), check("dialog", None), key("ctrl+a"), key("BackSpace"), check("query", ""),
                       check("total", 120), check("selected", "A little more room to think"), key("Escape"), wait(80), key("m"), check("dialog", "Move"),
                       click(600, 407), check("dialog", None), check("total", 119),
                       click(85, 398), check("folder", "Archive"), check("total", 1), shot("archived-message"))

    def test_appearance_toggle_and_calendar_with_mouse(self):
        # These are settled visual captures, not input-to-paint timing evidence.
        # First-paint scheduling after appearance changes remains R15/R63 work.
        self.mcp.batch(click(187, 867), check("tab", "Preferences"), wait(150), shot("preferences-light"),
                       click(690, 366), check("dark", True), wait(150), shot("preferences-dark"),
                       click(90, 159), check("tab", "Calendar"), wait(2000), shot("calendar-dark"),
                       key("ctrl+comma"), check("tab", "Preferences"), click(399, 366), check("dark", False),
                       click(90, 159), check("tab", "Calendar"), wait(2000), shot("calendar-light"))

    def test_calendar_navigation_repaints_after_preferences(self):
        # Disable the periodic UI tick in one fixture: pixels must update from
        # navigation itself, without a following mouse move/resize to wake it.
        # This is a correctness regression, not an idle-host latency benchmark.
        for idle in (False, True):
            result = self.mcp.call("desktop.start", idle_navigation=idle)
            directory = Path(result["artifacts"])
            print(f"Navigation pixels (timer disabled={idle}): {directory}", flush=True)
            for dark, x in ((True, 690), (False, 399)):
                with self.subTest(idle_navigation=idle, dark=dark):
                    name = "dark" if dark else "light"
                    self.mcp.batch(click(187, 867), check("tab", "Preferences"), wait(150),
                                   click(x, 366), check("dark", dark), check("preferences_saved", True),
                                   wait(200), shot(f"before-calendar-{name}"),
                                   click(90, 159), check("tab", "Calendar"), wait(150),
                                   shot(f"after-calendar-{name}"))
                    def pixels(prefix):
                        return subprocess.check_output([
                            "convert", str(directory / f"{prefix}-calendar-{name}.webp"),
                            "-crop", "900x650+250+210", "+repage", "-depth", "8", "rgb:-"])
                    before, after = pixels("before"), pixels("after")
                    self.assertEqual(len(before), len(after))
                    self.assertGreater(sum(abs(a-b) for a,b in zip(before,after))/len(before), 1.,
                                       "Calendar state changed but Preferences remained visible")

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

    def test_search_mouse_focus_blocks_default_and_remapped_delete_chords(self):
        self.mcp.batch(click(415,154), type_text("invoice"), check("total",1), key("ctrl+d"),
                       key("ctrl+a"), key("BackSpace"), check("total",120), check("action_toast",None),
                       key("Escape"), key("ctrl+comma"), check("tab","Preferences"), wait(80),
                       click(645,156), check("settings_tab","Shortcuts"),
                       {"type":"hover","x":1200,"y":700},{"type":"scroll","amount":30},wait(150),
                       click(920,540),key("alt+d"),check("shortcuts.Delete","Alt+D"),check("preferences_saved",True),
                       key("ctrl+1"),check("tab","Mail"),wait(80),
                       click(415,154),type_text("invoice"),check("total",1),key("alt+d"),
                       key("ctrl+a"),key("BackSpace"),check("total",120),check("action_toast",None),
                       key("Escape"),key("alt+d"),check("total",119),check("action_toast.label","Deleted 1 message"),
                       shot("remapped-delete-respects-search-focus"))

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
                       click(988, 600), check("shortcuts.Inbox", ""), check("shortcuts.Delete", "Mod+D"), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), click(100, 537), check("folder", "Projects"),
                       key("i"), wait(80), check("folder", "Projects"),
                       key("ctrl+comma"), check("tab", "Preferences"),
                       {"type": "hover", "x": 1200, "y": 700}, {"type": "scroll", "amount": 30}, wait(120),
                       click(920, 600), key("alt+i"), check("shortcuts.Inbox", "Alt+I"), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), click(100, 537), check("folder", "Projects"), key("alt+i"), check("folder", "INBOX"))

    def test_mail_navigation_clears_old_folder_highlight(self):
        for unified, appearance, dark in ((True, 399, False), (False, 690, True)):
            self.mcp.call("desktop.start", long_folders=True)
            self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), wait(150),
                           click(appearance, 366), check("dark", dark))
            if not unified:
                self.mcp.batch(click(286, 737), check("unified", False))
            self.mcp.batch(check("preferences_saved", True), key("ctrl+1"), check("tab", "Mail"), wait(150),
                           click(100, 537 if unified else 491), check("folder", "Projects"),
                           check("sidebar_focus", True), shot(f"folder-focused-{dark}"),
                           click(85, 115), check("folder", "INBOX"),
                           check("account", None if unified else "preview-work"),
                           check("sidebar_focus", False), check("mail_selection.list_focus", True), wait(150),
                           shot(f"mail-clears-folder-focus-{dark}"),
                           key("Tab"), check("sidebar_focus", True), key("Return"), check("folder", "INBOX"),
                           shot(f"inbox-focus-after-mail-{dark}"))

    def test_secondary_shortcut_remap_conflict_and_disable(self):
        self.mcp.batch(key("ctrl+comma"), check("tab", "Preferences"), click(645, 156),
                       check("settings_tab", "Shortcuts"), shot("two-shortcut-slots"),
                       click(1100, 350), key("Delete"), check("notice", "assigned more than once", "contains"),
                       key("alt+m"), check("shortcut_secondary.Move", "Alt+M"), check("preferences_saved", True),
                       key("ctrl+1"), check("tab", "Mail"), key("alt+m"), check("dialog", "Move"), check("focused_input", "folder-search"), key("Escape"), check("dialog", None),
                       key("m"), check("dialog", "Move"), check("focused_input", "folder-search"), key("Escape"), check("dialog", None),
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
                       key("ctrl+1"), check("tab", "Mail"), wait(150), shot("preferences-return-before-resize"),
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
                       click(98, 517), check("dialog", "Compose"),
                       check("fields.to", "friend@example.com"), check("fields.subject", "Meet me Monday"),
                       check("editor", "A message written with the mouse and keyboard.", "contains"), shot("reopened-draft"))

    def test_drafts_collapse_context_cancel_and_discard(self):
        self.mcp.batch(key("c"), check("dialog", "Compose"), wait(80),
                       click(650, 362), type_text("First draft to keep"), check("draft_count", 1),
                       key("Escape"), check("dialog", None),
                       key("c"), check("dialog", "Compose"), wait(80),
                       click(650, 362), type_text("Second draft to discard"), check("draft_count", 2),
                       key("Escape"), check("dialog", None), wait(80), shot("drafts-expanded"),
                       click(98, 478), check("drafts_collapsed", True), check("saved_drafts_collapsed", True),
                       shot("drafts-collapsed"), key("ctrl+2"), check("tab", "Calendar"),
                       key("ctrl+1"), check("tab", "Mail"), check("drafts_collapsed", True),
                       click(98, 478), check("drafts_collapsed", False), check("saved_drafts_collapsed", False),
                       click(1400, 36), check("busy", "sync", "contains"), {"type":"click", "x":100,"y":553,"button":3},
                       check("draft_context", None, "ne"), check("busy", []),
                       check("draft_context", None, "ne"), shot("draft-context-after-refresh"),
                       key("Down"), key("Return"), check("dialog", "DiscardDraft"), shot("discard-review-light"),
                       key("n"), check("dialog", None), check("draft_count", 2),
                       {"type":"click", "x":100,"y":553,"button":3}, check("draft_context", None,"ne"),
                       click(190, 610), check("dialog", "DiscardDraft"), key("Return"),
                       check("dialog", None), check("draft_count", 1),
                       check("draft_rows.0.1", "First draft to keep"), shot("draft-discarded"),
                       click(98, 517), check("dialog", "Compose"), check("fields.subject", "First draft to keep"))

    def test_draft_bin_cancel_failure_retry_and_compact_dark_review(self):
        result = self.mcp.call("desktop.start", discard_failure_once=True)
        fixture = Path(result["artifacts"]) / "discard attachment.txt"
        fixture.write_text("Cached bytes to remove with the draft")
        self.mcp.batch(key("c"), check("dialog", "Compose"), wait(80),
                       click(650,362), type_text("Draft with an attachment"),
                       click(650,485), type_text("Do not lose this on a failed discard."),
                       click(583,720), {"type":"choose_file","path":str(fixture)},
                       check("draft_attachments.0.name", fixture.name), check("draft_io",False), wait(150),
                       click(916,741), check("dialog","DiscardDraft"), shot("discard-attachment-review"),
                       key("Escape"), check("dialog","Compose"),
                       check("editor","Do not lose this on a failed discard.","contains"),
                       click(916,741), check("dialog","DiscardDraft"), key("y"),
                       check("notice","Preview storage failure","contains"), check("discard_pending",False),
                       check("draft_count",1), shot("discard-failure-keeps-draft"), key("Escape"),
                       check("dialog","Compose"), check("draft_attachments.0.name",fixture.name), wait(80),
                       click(916,716), check("dialog","DiscardDraft"), key("Return"),
                       check("draft_count",0), check("dialog",None), check("draft_attachments",[]))
        self.mcp.call("desktop.start", width=900,height=640)
        self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"), click(563,366),check("dark",True),
                       key("ctrl+1"),check("tab","Mail"), key("c"),check("dialog","Compose"),wait(80),
                       click(450,257),type_text("Compact draft"),check("draft_count",1),
                       click(646,527),check("dialog","DiscardDraft"),shot("discard-review-dark-compact"),
                       key("Escape"),check("dialog","Compose"),check("fields.subject","Compact draft"))

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
        self.mcp.batch(click(98, 517), check("dialog", "Compose"), check("fields.cc", "copy@example.com"),
                       check("fields.bcc", "hidden@example.com"), check("draft_attachments.0.name", second.name),
                       check("editor", "Please read the attached notes.", "contains"), shot("reopened-attachments"),
                       click(465, 790), check("notice", "Sending is disabled in preview", "contains"),
                       check("dialog", "Compose"), check("draft_attachments.1.name", third.name), shot("send-failure-keeps-draft"),
                       key("Escape"), check("dialog", None), key("ctrl+comma"), check("tab", "Preferences"),
                       click(690, 366), check("dark", True), key("ctrl+1"), check("tab", "Mail"),
                       click(98, 517), check("dialog", "Compose"), shot("composer-dark-attachments"),
                       click(583, 790), {"type": "choose_file"}, check("draft_io", False),
                       check("draft_attachments.1.name", third.name), key("Escape"), check("dialog", None))


    def test_reply_all_mouse_and_remappable_shortcut(self):
        self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("prototype"),
                       check("total", 1), key("Escape"), check("html_ready", True), wait(100), click(766, 784), check("dialog", "Compose"),
                       check("fields.to", "Daniel Park <team@example.com>, colleague@example.com"),
                       check("fields.cc", "copy@example.com"), check("fields.bcc", ""),
                       check("draft_in_reply_to", "<prototype@example.com>"), shot("reply-all-mouse"),
                       key("Escape"), check("dialog", None), key("r"), check("dialog", "Compose"),
                       check("fields.to", "Daniel Park <team@example.com>"), check("fields.cc", ""),
                       key("Escape"), check("dialog", None), key("shift+r"), check("dialog", "Compose"),
                       check("fields.cc", "copy@example.com"), shot("reply-all-keyboard"))

    def test_composer_typing_does_not_leave_glyphs_below_editor(self):
        from PIL import Image, ImageChops
        for dark in (False, True):
            result = self.mcp.call("desktop.start",width=900,height=640)
            if dark:
                self.mcp.batch(key("ctrl+comma"),check("tab","Preferences"),wait(100),click(563,366),check("dark",True),key("ctrl+1"))
            self.mcp.batch(key("r"),check("dialog","Compose"),click(450,370),key("ctrl+a"),
                           {"type":"paste","text":"Hello,\n\nHere is my reply.\n\n" + "\n".join(f"> Quoted message line {i:02}: keep the editor edge clean." for i in range(40))},
                           key("ctrl+Home"),type_text("My reply"),key("Return"),type_text("Thank you"),key("Return"),
                           key("Return"),key("Return"),wait(100),shot(f"composer-typed-edge-{dark}"),
                           {"type":"resize","width":901,"height":640},check("window_size",[901.,640.]),
                           {"type":"resize","width":900,"height":640},check("window_size",[900.,640.]),
                           wait(100),shot(f"composer-repainted-edge-{dark}"))
            directory = Path(result["artifacts"])
            before = Image.open(directory/f"composer-typed-edge-{dark}.webp").convert("RGB")
            after = Image.open(directory/f"composer-repainted-edge-{dark}.webp").convert("RGB")
            # Bottom editor padding and the gap above the compose action bar.
            region = (150,490,740,526)
            difference = ImageChops.difference(before.crop(region),after.crop(region))
            changed = sum(max(p)>32 for p in difference.getdata())
            self.assertLess(changed,60,f"Editor left {changed} stale pixels under its text viewport")
            padding = before.crop((152,504,680,511))
            background = before.getpixel((700,506))
            escaped = sum(max(abs(p[i]-background[i]) for i in range(3))>40 for p in padding.getdata())
            self.assertLess(escaped,8,f"{escaped} glyph pixels escaped into the editor's bottom padding")

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
                       check("total", 25), key("Escape"), check("conversation_total", 25),
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
                       click(100, 517), check("dialog", "Compose"),
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
                           check("tab", "Preferences"), wait(80),
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
        for index, x in enumerate([1115, 1218, 1324]):
            if index: self.mcp.call("desktop.start")
            self.mcp.batch(key("ctrl+k"), check("focused_input", "search"), type_text("prottoype"), check("total", 1), key("Escape"),
                           check("reply_count", 1), check("attachment_count", 4), check("images_allowed", False),
                           check("html_ready", True), wait(100), shot("prototype-html-layout"))
            self.toggle_html_quotes(False)
            self.mcp.batch(wait(100), shot("expanded-html-reply"))
            self.toggle_html_quotes(True)
            self.mcp.batch(click(740, 243), check("dialog", "Sender"), shot("sender-details"), key("Escape"), check("dialog", None),
                           click(x, 376), check("images_allowed", True), wait(100), shot(f"image-exception-{index}"))
        self.mcp.batch(click(770,320), check("html_formatted", False), check("reader_text_ready", True),
                       wait(100), shot("plain-quoted-history"), click(740,419), check("expanded_replies", 0, "contains"),
                       wait(100), shot("plain-quoted-history-expanded"))
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
