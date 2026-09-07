import importlib.util
from pathlib import Path
import unittest
from unittest.mock import Mock
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
