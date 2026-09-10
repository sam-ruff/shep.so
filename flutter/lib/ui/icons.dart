import 'package:flutter/material.dart';

/// Stroked 24-unit icons matching the desktop client's embedded SVG set.
const Map<String, List<IconShape>> shepIconShapes = {
  'image': [
    IconRect(3, 3, 18, 18, 2),
    IconCircle(8, 8, 2),
    IconPath('m21 15-5-5L5 21'),
  ],
  'mail-open': [
    IconPath('m3 9 9-6 9 6v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9Z'),
    IconPath('m3 9 9 6 9-6M3 20l6-7m12 7-6-7'),
  ],
  'mail': [IconRect(3, 5, 18, 14, 2), IconPath('m3 6 9 7 9-7')],
  'inbox': [
    IconPath('M4 4h16l2 11v5H2v-5L4 4Z'),
    IconPath('M2 15h6l2 3h4l2-3h6'),
  ],
  'calendar': [
    IconRect(3, 5, 18, 16, 2),
    IconPath('M16 3v4M8 3v4M3 11h18M8 15h2M14 15h2'),
  ],
  'compose': [
    IconPath(
      'm15 4 5 5M4 20l4-1L21 6a2 2 0 0 0-3-3L5 16l-1 4ZM13 4H5a2 2 0 0 0-2 2v14a1 1 0 0 0 1 1h14a2 2 0 0 0 2-2v-6',
    ),
  ],
  'flag': [IconPath('M5 21V4c5-4 9 4 14 0v11c-5 4-9-4-14 0')],
  'star': [
    IconPath(
      'm12 3 2.8 5.8 6.4.9-4.6 4.5 1.1 6.3-5.7-3-5.7 3 1.1-6.3-4.6-4.5 6.4-.9L12 3Z',
    ),
  ],
  'send': [IconPath('m22 2-7 20-4-9-9-4L22 2ZM22 2 11 13')],
  'file': [IconPath('M14 2H5v20h14V7l-5-5ZM14 2v6h5M8 13h8M8 17h6')],
  'archive': [IconRect(3, 3, 18, 4, 1), IconPath('M5 7v14h14V7M10 11h4')],
  'trash': [IconPath('M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7')],
  'folder': [IconPath('M3 7V4h6l2 3h10v13H3V7Z')],
  'move': [IconPath('M3 7V4h6l2 3h10v13H3V7ZM8 14h8m-3-3 3 3-3 3')],
  'plus': [IconPath('M12 5v14M5 12h14')],
  'search': [IconCircle(10.5, 10.5, 6.5), IconPath('m16 16 5 5')],
  'down': [IconPath('m6 9 6 6 6-6')],
  'chevron-down': [IconPath('m5 9 7 7 7-7')],
  'chevron': [IconPath('m9 5 7 7-7 7')],
  'left': [IconPath('m15 5-7 7 7 7')],
  'sync': [
    IconPath(
      'M20 9a8.25 8.25 0 0 0-14-3L3 9m0-6v6h6M4 15a8.25 8.25 0 0 0 14 3l3-3m0 6v-6h-6',
    ),
  ],
  'settings': [
    IconPath(
      'm9 3 1-1h4l1 3 3 1 3 3-1 3 1 3-3 3-3 1-1 3h-4l-1-3-3-1-3-3 1-3-1-3 3-3 3-1V3Z',
    ),
    IconCircle(12, 12, 3),
  ],
  'shield': [
    IconPath('m12 2 9 4v6c0 5-9 10-9 10S3 17 3 12V6l9-4Z'),
    IconPath('m8 12 3 3 5-6'),
  ],
  'cloud': [IconPath('M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-2 6 6 0 0 1 0 12H6Z')],
  'reply': [IconPath('m9 4-6 6 6 6M3 10h11a7 7 0 0 1 7 7v3')],
  'reply-all': [IconPath('m8 5-5 5 5 5m5-10-5 5 5 5M8 10h6a7 7 0 0 1 7 7v3')],
  'print': [
    IconPath('M6 9V3h12v6M6 18H3V9h18v9h-3M6 14h12v7H6z'),
    IconPath('M17 11h1'),
  ],
  'forward': [IconPath('m15 4 6 6-6 6m6-6H10a7 7 0 0 0-7 7v3')],
  'clip': [
    IconPath(
      'm21 11-9 9a6 6 0 0 1-8-8L14 2a4 4 0 0 1 6 6L10 18a2 2 0 0 1-3-3l9-9',
    ),
  ],
  'close': [IconPath('m6 6 12 12M6 18 18 6')],
  'check': [IconPath('m5 12 4 4L19 6')],
  'sun': [
    IconCircle(12, 12, 4),
    IconPath(
      'M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1 1M18 18l1 1M5 19l1-1M18 6l1-1',
    ),
  ],
  'moon': [IconPath('M21 13A9 9 0 0 1 11 3 9 9 0 1 0 21 13Z')],
  'keyboard': [
    IconRect(2, 5, 20, 14, 2),
    IconPath('M6 9h1m3 0h1m3 0h1m3 0h1M6 12h1m3 0h1m3 0h1m3 0h1M7 16h10'),
  ],
  'copy': [
    IconRect(8, 8, 12, 13, 2),
    IconPath('M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h3'),
  ],
  'download': [IconPath('M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5')],
  'up': [IconPath('m6 15 6-6 6 6')],
  'clock': [IconCircle(12, 12, 9), IconPath('M12 7v5l3 2')],
  // Mobile-only controls keep the same stroke language.
  'more': [
    IconCircle(5, 12, 1.1),
    IconCircle(12, 12, 1.1),
    IconCircle(19, 12, 1.1),
  ],
  'menu': [IconPath('M3 6h18M3 12h18M3 18h18')],
  'sort': [IconPath('m21 16-4 4-4-4M17 20V4M3 8l4-4 4 4M7 4v16')],
  'at': [
    IconCircle(12, 12, 4),
    IconPath('M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8'),
  ],
  'check-circle': [IconCircle(12, 12, 9), IconPath('m8 12 3 3 5-6')],
  'minus-circle': [IconCircle(12, 12, 9), IconPath('M8 12h8')],
  'info': [IconCircle(12, 12, 9), IconPath('M12 16v-4M12 8h.01')],
  'lock': [IconRect(3, 11, 18, 11, 2), IconPath('M7 11V7a5 5 0 0 1 10 0v4')],
  'link': [
    IconPath(
      'M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1',
    ),
  ],
  'eye': [
    IconPath('M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7Z'),
    IconCircle(12, 12, 3),
  ],
  'user': [
    IconCircle(12, 12, 10),
    IconCircle(12, 10, 3),
    IconPath('M7 20.66V19a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v1.66'),
  ],
  'type': [IconPath('M4 7V4h16v3M9 20h6M12 4v16')],
  'key-off': [
    IconPath('m2 2 20 20M15.5 7.5 19 4l1 1-3.5 3.5M12 12l-7 7v3h3l7-7'),
    IconCircle(16, 8, 3),
  ],
  'cloud-off': [
    IconPath(
      'm2 2 20 20M9 5a7 7 0 0 1 9 6 6 6 0 0 1 4 8M6 18a5 5 0 0 1-1-10h1M6 18h10',
    ),
  ],
  'cloud-check': [
    IconPath('M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-2 6 6 0 0 1 0 12H6Z'),
    IconPath('m9 12 2 2 4-4'),
  ],
  'cloud-upload': [
    IconPath('M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-2 6 6 0 0 1 0 12h-2'),
    IconPath('M12 12v9m-4-5 4-4 4 4'),
  ],
  'swipe-left': [IconPath('m6 12 4 4m-4-4 4-4M6 12h12')],
  'swipe-right': [IconPath('m18 12-4 4m4-4-4-4M6 12h12')],
  'outbox': [
    IconPath('M4 4h16l2 11v5H2v-5L4 4Z'),
    IconPath('M12 15V7m-3 3 3-3 3 3'),
  ],
};

sealed class IconShape {
  const IconShape();
  void addTo(Path path);
}

class IconPath extends IconShape {
  const IconPath(this.data);
  final String data;
  @override
  void addTo(Path path) => _appendSvgPath(path, data);
}

class IconRect extends IconShape {
  const IconRect(this.x, this.y, this.width, this.height, this.radius);
  final double x, y, width, height, radius;
  @override
  void addTo(Path path) => path.addRRect(
    RRect.fromRectAndRadius(
      Rect.fromLTWH(x, y, width, height),
      Radius.circular(radius),
    ),
  );
}

class IconCircle extends IconShape {
  const IconCircle(this.cx, this.cy, this.r);
  final double cx, cy, r;
  @override
  void addTo(Path path) =>
      path.addOval(Rect.fromCircle(center: Offset(cx, cy), radius: r));
}

final RegExp _token = RegExp(r'[MmLlHhVvCcSsQqTtAaZz]|-?\d*\.?\d+(?:e-?\d+)?');

/// Appends SVG path data (absolute and relative commands, including arcs).
void _appendSvgPath(Path path, String data) {
  final tokens = _token.allMatches(data).map((m) => m.group(0)!).toList();
  var index = 0;
  var command = '';
  var x = 0.0, y = 0.0, startX = 0.0, startY = 0.0;
  var controlX = 0.0, controlY = 0.0;
  double next() => double.parse(tokens[index++]);
  bool numberNext() => index < tokens.length && !_isCommand(tokens[index]);
  while (index < tokens.length) {
    if (_isCommand(tokens[index])) command = tokens[index++];
    final relative = command == command.toLowerCase();
    final dx = relative ? x : 0.0, dy = relative ? y : 0.0;
    switch (command.toUpperCase()) {
      case 'M':
        x = next() + dx;
        y = next() + dy;
        path.moveTo(x, y);
        startX = x;
        startY = y;
        command = relative ? 'l' : 'L';
      case 'L':
        x = next() + dx;
        y = next() + dy;
        path.lineTo(x, y);
      case 'H':
        x = next() + dx;
        path.lineTo(x, y);
      case 'V':
        y = next() + dy;
        path.lineTo(x, y);
      case 'C':
        final x1 = next() + dx, y1 = next() + dy;
        controlX = next() + dx;
        controlY = next() + dy;
        x = next() + dx;
        y = next() + dy;
        path.cubicTo(x1, y1, controlX, controlY, x, y);
      case 'S':
        final x1 = 2 * x - controlX, y1 = 2 * y - controlY;
        controlX = next() + dx;
        controlY = next() + dy;
        x = next() + dx;
        y = next() + dy;
        path.cubicTo(x1, y1, controlX, controlY, x, y);
      case 'Q':
        controlX = next() + dx;
        controlY = next() + dy;
        x = next() + dx;
        y = next() + dy;
        path.quadraticBezierTo(controlX, controlY, x, y);
      case 'T':
        controlX = 2 * x - controlX;
        controlY = 2 * y - controlY;
        x = next() + dx;
        y = next() + dy;
        path.quadraticBezierTo(controlX, controlY, x, y);
      case 'A':
        final rx = next(), ry = next(), rotation = next();
        final largeArc = next() != 0, sweep = next() != 0;
        x = next() + dx;
        y = next() + dy;
        path.arcToPoint(
          Offset(x, y),
          radius: Radius.elliptical(rx, ry),
          rotation: rotation,
          largeArc: largeArc,
          clockwise: sweep,
        );
      case 'Z':
        path.close();
        x = startX;
        y = startY;
      default:
        return;
    }
    if (command.toUpperCase() != 'C' &&
        command.toUpperCase() != 'S' &&
        command.toUpperCase() != 'Q' &&
        command.toUpperCase() != 'T') {
      controlX = x;
      controlY = y;
    }
    if (command.toUpperCase() == 'Z' && numberNext()) command = 'L';
  }
}

bool _isCommand(String token) => _letter.hasMatch(token);

final RegExp _letter = RegExp(r'^[A-Za-z]$');

/// A desktop-style stroked icon; colour and size follow the ambient IconTheme.
class ShepIcon extends StatelessWidget {
  const ShepIcon(
    this.name, {
    super.key,
    this.size,
    this.color,
    this.strokeWidth = 1.6,
  });
  final String name;
  final double? size;
  final Color? color;
  final double strokeWidth;

  @override
  Widget build(BuildContext context) {
    final theme = IconTheme.of(context);
    final side = size ?? theme.size ?? 18;
    final paint =
        (color ?? theme.color ?? Theme.of(context).colorScheme.onSurfaceVariant)
            .withValues(alpha: theme.opacity ?? 1);
    return SizedBox(
      width: side,
      height: side,
      child: CustomPaint(
        painter: _IconPainter(
          shepIconShapes[name] ?? shepIconShapes['mail']!,
          paint,
          strokeWidth,
        ),
      ),
    );
  }
}

class _IconPainter extends CustomPainter {
  _IconPainter(this.shapes, this.color, this.strokeWidth);
  final List<IconShape> shapes;
  final Color color;
  final double strokeWidth;

  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.shortestSide / 24;
    canvas.scale(scale);
    final path = Path();
    for (final shape in shapes) {
      shape.addTo(path);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = strokeWidth
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(_IconPainter old) =>
      old.shapes != shapes ||
      old.color != color ||
      old.strokeWidth != strokeWidth;
}
