import importlib.util
from pathlib import Path
import unittest
import tempfile
from unittest.mock import Mock, patch
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location('picker', Path(__file__).resolve().parents[1] / 'scripts/clients/android_compose_fixture.py')
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)

class PickerRecoveryTest(unittest.TestCase):
    def check_dialog(self, title, wait=True):
        controller = object.__new__(picker.AndroidPicker)
        controller.tap = Mock()
        nodes = [ET.Element('node', {'text': title}), ET.Element('node', {'text': 'Close app'})]
        if wait:
            nodes.append(ET.Element('node', {'text': 'Wait'}))
        result = controller.wait_for_system_ui(nodes)
        return result, controller.tap, nodes

    def test_system_dialog_waits_without_closing_any_app(self):
        result, tap, nodes = self.check_dialog("System UI isn't responding")
        self.assertTrue(result)
        tap.assert_called_once_with(nodes[-1])

    def test_application_failure_is_not_dismissed_as_system_ui(self):
        result, tap, _ = self.check_dialog("Shep isn't responding")
        self.assertFalse(result)
        tap.assert_not_called()

    def test_missing_wait_control_does_not_select_close(self):
        result, tap, _ = self.check_dialog("System UI isn't responding", wait=False)
        self.assertFalse(result)
        tap.assert_not_called()


    def test_window_refuses_an_old_dump_when_uiautomator_does_not_write(self):
        controller = object.__new__(picker.AndroidPicker)
        stale = b'<hierarchy><node text="SAVE" package="com.google.android.documentsui"/></hierarchy>'
        remote = {'data': stale}
        fresh = {'enabled': False}
        def adb(*args, **kwargs):
            if args[:3] == ('shell', 'rm', '-f'):
                remote['data'] = b''
            elif args[:3] == ('shell', 'uiautomator', 'dump'):
                if fresh['enabled']:
                    remote['data'] = b'<hierarchy><node text="Current file"/></hierarchy>'
                return b'ERROR: could not get idle state.' if not fresh['enabled'] else b'UI hierarchy dumped'
            elif args[:2] == ('exec-out', 'cat'):
                return remote['data']
            return b''
        controller.adb = Mock(side_effect=adb)
        with tempfile.TemporaryDirectory() as directory, patch.object(picker, 'OUTPUT', Path(directory)):
            self.assertEqual(controller.window(), [])
            self.assertEqual((Path(directory)/'last-window.xml').read_bytes(), b'')
            fresh['enabled'] = True
            self.assertEqual([node.get('text') for node in controller.window()], ['Current file'])
