"""Read presented pixels and inject one real click on an owned X11 test window.

No app state is changed through the observation file. Setup and reference-image
preparation are outside the measured interval; the measurement ends at pixels,
not at a backend-ready notification.
"""
import ctypes
import ctypes.util
import math
import time


def validate_points(points, width, height):
    if not isinstance(points, list) or not 8 <= len(points) <= 128:
        raise ValueError("Pixel probes require 8–128 reference points.")
    for point in points:
        if not isinstance(point, list) or len(point) != 5 or any(type(v) is not int for v in point):
            raise ValueError("Pixel probes use integer [x, y, r, g, b] points.")
        x, y, *rgb = point
        if not 0 <= x < width or not 0 <= y < height or any(not 0 <= v <= 255 for v in rgb):
            raise ValueError("Pixel reference is outside the owned window or RGB range.")


class Window:
    def __init__(self, display_name, window):
        self.x = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
        self.xtest = ctypes.CDLL(ctypes.util.find_library("Xtst") or "libXtst.so.6")
        self.x.XOpenDisplay.argtypes, self.x.XOpenDisplay.restype = [ctypes.c_char_p], ctypes.c_void_p
        self.x.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self.x.XFlush.argtypes = [ctypes.c_void_p]
        self.x.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                                     ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
        self.x.XGetImage.restype = ctypes.c_void_p
        self.x.XGetPixel.argtypes, self.x.XGetPixel.restype = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int], ctypes.c_ulong
        self.x.XDestroyImage.argtypes = [ctypes.c_void_p]
        self.x.XGetGeometry.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
            *([ctypes.POINTER(ctypes.c_uint)]*4)]
        self.xtest.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.display = self.x.XOpenDisplay(display_name.encode())
        self.window = int(window)
        if not self.display:
            raise RuntimeError("The owned fixture display is unavailable.")

    def close(self):
        if self.display:
            self.x.XCloseDisplay(self.display)
            self.display = None

    def dimensions(self):
        root, x, y = ctypes.c_ulong(), ctypes.c_int(), ctypes.c_int()
        width, height, border, depth = (ctypes.c_uint() for _ in range(4))
        if not self.x.XGetGeometry(self.display, self.window, ctypes.byref(root), ctypes.byref(x), ctypes.byref(y),
                                  ctypes.byref(width), ctypes.byref(height), ctypes.byref(border), ctypes.byref(depth)):
            raise RuntimeError("The owned window's current geometry is unavailable.")
        return width.value, height.value

    def matched(self, points, tolerance=8):
        left, top = min(p[0] for p in points), min(p[1] for p in points)
        width, height = max(p[0] for p in points)-left+1, max(p[1] for p in points)-top+1
        data = self.x.XGetImage(self.display, self.window, left, top, width, height,
                               ctypes.c_ulong(-1).value, 2)  # ZPixmap on the harness's RGB24 Xvfb
        if not data:
            raise RuntimeError("The owned window could not be sampled.")
        try:
            matched = 0
            for x, y, red, green, blue in points:
                pixel = self.x.XGetPixel(data, x-left, y-top)
                actual = ((pixel >> 16) & 255, (pixel >> 8) & 255, pixel & 255)
                matched += all(abs(a-b) <= tolerance for a,b in zip(actual, (red, green, blue)))
            return matched / len(points)
        finally:
            self.x.XDestroyImage(data)

    def rgb(self, width, height):
        class XImage(ctypes.Structure):
            _fields_ = [(name, kind) for name, kind in (
                ("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int),
                ("format", ctypes.c_int), ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int),
                ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
                ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int), ("bytes_per_line", ctypes.c_int),
                ("bits_per_pixel", ctypes.c_int), ("red_mask", ctypes.c_ulong),
                ("green_mask", ctypes.c_ulong), ("blue_mask", ctypes.c_ulong))]
        data = self.x.XGetImage(self.display, self.window, 0, 0, width, height,
                               ctypes.c_ulong(-1).value, 2)
        if not data:
            raise RuntimeError("The owned window could not be sampled.")
        try:
            image = ctypes.cast(data, ctypes.POINTER(XImage)).contents
            if (image.byte_order, image.bits_per_pixel, image.red_mask, image.green_mask, image.blue_mask) != (0,32,0xff0000,0xff00,0xff):
                raise RuntimeError("Pixel measurement requires the harness's little-endian RGB24 Xvfb.")
            raw = ctypes.string_at(image.data, image.bytes_per_line*height)
            if image.bytes_per_line != width*4:
                raw = b"".join(raw[y*image.bytes_per_line:y*image.bytes_per_line+width*4] for y in range(height))
            rgb = bytearray(width*height*3)
            rgb[0::3], rgb[1::3], rgb[2::3] = raw[2::4], raw[1::4], raw[0::4]
            return rgb
        finally:
            self.x.XDestroyImage(data)

    def click_until_visible(self, points, timeout_ms=5000):
        if type(timeout_ms) is not int or not 1 <= timeout_ms <= 10000:
            raise ValueError("Pixel measurement deadline must be 1–10000 milliseconds.")
        before = self.matched(points)
        if before >= .97:
            raise ValueError("The reference pixels are already visible; choose a different message first.")
        self.x.XSync(self.display, False)
        started = time.perf_counter()
        wall_started = time.time()
        self.xtest.XTestFakeButtonEvent(self.display, 1, True, 0)
        self.xtest.XTestFakeButtonEvent(self.display, 1, False, 0)
        self.x.XFlush(self.display)
        polls = 0
        previous = started
        max_interval = 0.
        while True:
            match = self.matched(points)
            now = time.perf_counter()
            polls += 1
            max_interval = max(max_interval, now-previous)
            previous = now
            if match >= .97:
                return {"input_to_pixels_ms": (now-started)*1000, "match": match,
                        "started_at": wall_started,
                        "before_match": before, "polls": polls, "max_poll_interval_ms": max_interval*1000}
            if (now-started)*1000 >= timeout_ms:
                raise TimeoutError(f"HTML reference pixels did not appear ({match:.1%} matched).")
            time.sleep(.002)


def reference_points(rgb, width, height, bounds):
    """Pick text/edge pixels across the visible body, not just its background."""
    if len(rgb) != width*height*3:
        raise ValueError("The RGB reference dimensions do not match its bytes.")
    x, y, w, h = bounds
    left, top = max(0, math.ceil(x)+4), max(0, math.ceil(y)+4)
    right, bottom = min(width-2, math.floor(x+w)-4), min(height-2, math.floor(y+h)-4)
    if right-left < 32 or bottom-top < 32:
        raise ValueError("The HTML body must be visible to prepare pixel references.")
    def pixel(px, py):
        offset = (py*width+px)*3
        return rgb[offset:offset+3]
    points = []
    for row in range(8):
        for col in range(8):
            x0, x1 = left+(right-left)*col//8, left+(right-left)*(col+1)//8
            y0, y1 = top+(bottom-top)*row//8, top+(bottom-top)*(row+1)//8
            best, selected = -1, (x0, y0)
            for py in range(y0, y1, 2):
                for px in range(x0, x1, 2):
                    score = sum(abs(a-b) for a,b in zip(pixel(px, py), pixel(px+1, py+1)))
                    if score > best:
                        best, selected = score, (px, py)
            px, py = selected
            points.append([px, py, *pixel(px, py)])
    return points
