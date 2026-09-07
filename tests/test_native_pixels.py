import unittest
from unittest.mock import Mock, patch
from scripts import native_pixels as pixels


class PixelMeasurementTests(unittest.TestCase):
    def test_rejects_invalid_or_outside_references(self):
        valid = [[10, 20, 30, 40, 50]]*8
        pixels.validate_points(valid, 100, 100)
        for points in ([], valid[:7], valid*17, [[True,20,30,40,50]]*8,
                       [[100,20,30,40,50]]*8, [[10,20,-1,40,50]]*8):
            with self.assertRaises(ValueError):
                pixels.validate_points(points, 100, 100)

    def test_already_visible_reference_never_injects_click(self):
        window = object.__new__(pixels.Window)
        window.matched, window.xtest = Mock(return_value=1.), Mock()
        with self.assertRaisesRegex(ValueError, "already visible"):
            window.click_until_visible([])
        window.xtest.XTestFakeButtonEvent.assert_not_called()

    def test_measurement_finishes_only_after_presented_reference_matches(self):
        window = object.__new__(pixels.Window)
        window.display, window.x, window.xtest = 123, Mock(), Mock()
        window.matched = Mock(side_effect=[0., .5, 1.])
        with patch.object(pixels.time, "perf_counter", side_effect=[10., 10.010, 10.022]), patch.object(pixels.time, "sleep"):
            result = window.click_until_visible([])
        self.assertAlmostEqual(result["input_to_pixels_ms"], 22.)
        self.assertEqual(result["polls"], 2)
        self.assertEqual(window.xtest.XTestFakeButtonEvent.call_count, 2)
        window.x.XFlush.assert_called_once_with(123)

    def test_reference_sampling_stays_in_body_and_prefers_text_edges(self):
        rgb = bytearray([255]*100*100*3)
        for y in range(10,90):
            for x in range(10,90,4):
                rgb[(y*100+x)*3:(y*100+x)*3+3] = b"\0\0\0"
        points = pixels.reference_points(rgb, 100, 100, [5,5,90,90])
        pixels.validate_points(points, 100, 100)
        self.assertEqual(len(points), 64)
        self.assertTrue(all(9 <= x < 91 and 9 <= y < 91 for x,y,*_ in points))
        self.assertGreater(sum(
            rgb[(y*100+x)*3:(y*100+x)*3+3] != rgb[((y+1)*100+x+1)*3:((y+1)*100+x+1)*3+3]
            for x,y,*_ in points), 40)
