import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../services.dart';
import 'theme.dart';

/// The big left-aligned title at the top of each tab.
class PageHeader extends StatelessWidget {
  const PageHeader(this.title, {super.key, this.actions = const []});

  final String title;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 12, 8, 8),
      child: Row(
        children: [
          Expanded(child: Text(title, style: Theme.of(context).textTheme.headlineMedium)),
          ...actions,
        ],
      ),
    );
  }
}

/// A titled group of rows on one card.
class Section extends StatelessWidget {
  const Section({super.key, this.title, required this.children, this.padding});

  final String? title;
  final List<Widget> children;
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: padding ?? const EdgeInsets.fromLTRB(16, 0, 16, 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (title != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(4, 0, 4, 8),
              child: Text(title!.toUpperCase(), style: Theme.of(context).textTheme.labelSmall),
            ),
          SurfaceCard(
            child: Column(
              children: [
                for (var i = 0; i < children.length; i++) ...[
                  if (i > 0) const Divider(indent: 16, endIndent: 16),
                  children[i],
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class SurfaceCard extends StatelessWidget {
  const SurfaceCard({super.key, required this.child, this.color = Palette.surface});

  final Widget child;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: color,
      clipBehavior: Clip.antiAlias,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(20),
        side: const BorderSide(color: Palette.border),
      ),
      child: child,
    );
  }
}

/// A settings row with an optional value on the right and a chevron when it opens a page.
class SettingRow extends StatelessWidget {
  const SettingRow({
    super.key,
    required this.title,
    this.icon,
    this.value,
    this.trailing,
    this.onTap,
  });

  final String title;
  final IconData? icon;
  final String? value;
  final Widget? trailing;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      minTileHeight: 56,
      leading: icon == null ? null : Icon(icon, size: 22),
      title: Text(title),
      onTap: onTap,
      trailing:
          trailing ??
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (value != null)
                ConstrainedBox(
                  // Large text leaves the title more room, so it never breaks inside a word.
                  constraints: BoxConstraints(
                    maxWidth:
                        MediaQuery.sizeOf(context).width *
                        (MediaQuery.textScalerOf(context).scale(1) > 1.5 ? 0.26 : 0.42),
                  ),
                  child: Text(
                    value!,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(color: Palette.muted, fontSize: 14),
                  ),
                ),
              if (onTap != null) ...[
                const SizedBox(width: 4),
                const Icon(Icons.chevron_right_rounded, color: Palette.faint),
              ],
            ],
          ),
    );
  }
}

/// A short centered message with the PickScribe mark, for empty lists.
class EmptyState extends StatelessWidget {
  const EmptyState(this.message, {super.key, this.action});

  final String message;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const BrandMark(size: 72, faded: true),
            const SizedBox(height: 16),
            Text(
              message,
              textAlign: TextAlign.center,
              style: const TextStyle(color: Palette.muted, fontSize: 15, height: 1.4),
            ),
            if (action != null) ...[const SizedBox(height: 20), action!],
          ],
        ),
      ),
    );
  }
}

/// The PickScribe mark: brackets around a waveform, with the ember dot where text lands.
class BrandMark extends StatelessWidget {
  const BrandMark({super.key, this.size = 48, this.faded = false});

  final double size;
  final bool faded;

  @override
  Widget build(BuildContext context) => CustomPaint(
    size: Size.square(size),
    painter: _MarkPainter(faded: faded),
  );
}

class _MarkPainter extends CustomPainter {
  const _MarkPainter({required this.faded});

  final bool faded;

  @override
  void paint(Canvas canvas, Size size) {
    // The mark is drawn on the 128-unit grid of pickscribe-mark-128.svg.
    canvas.scale(size.width / 128);
    final ink = faded ? Palette.faint : Palette.text;
    final bracket = Paint()
      ..color = ink
      ..style = PaintingStyle.stroke
      ..strokeWidth = 3
      ..strokeCap = StrokeCap.square;
    for (final points in const [
      [Offset(36, 52), Offset(36, 32), Offset(52, 32)],
      [Offset(36, 76), Offset(36, 96), Offset(52, 96)],
      [Offset(76, 32), Offset(92, 32), Offset(92, 52)],
      [Offset(76, 96), Offset(92, 96), Offset(92, 76)],
    ]) {
      canvas.drawPath(Path()..addPolygon(points, false), bracket);
    }
    final wave = Paint()
      ..color = ink.withValues(alpha: 0.55)
      ..strokeWidth = 2
      ..strokeCap = StrokeCap.round;
    canvas.drawCircle(const Offset(58, 64), 1, wave..style = PaintingStyle.fill);
    canvas.drawLine(const Offset(62, 58), const Offset(62, 70), wave);
    canvas.drawLine(const Offset(66, 52), const Offset(66, 76), wave);
    canvas.drawCircle(
      const Offset(86, 64),
      6,
      Paint()..color = faded ? Palette.faint : Palette.ember,
    );
  }

  @override
  bool shouldRepaint(_MarkPainter oldDelegate) => faded != oldDelegate.faded;
}

/// The bubble as it looks over other apps at rest, for the size and opacity preview. It
/// matches BubbleView.kt.
class BubbleGlyph extends StatelessWidget {
  const BubbleGlyph({super.key, required this.size, this.opacity = 1});

  final double size;
  final double opacity;

  @override
  Widget build(BuildContext context) => Opacity(
    opacity: opacity,
    child: CustomPaint(size: Size.square(size), painter: const _BubblePainter()),
  );
}

class _BubblePainter extends CustomPainter {
  const _BubblePainter();

  @override
  void paint(Canvas canvas, Size size) {
    final s = size.shortestSide;
    final center = size.center(Offset.zero);
    final radius = s / 2 - s * 0.06;
    canvas.drawCircle(center, radius, Paint()..color = const Color(0xFF0F0F11));
    canvas.drawCircle(
      center,
      radius,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = s * 0.04
        ..color = Palette.faint,
    );
    final ink = Paint()
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round
      ..strokeWidth = s * 0.07
      ..color = Palette.text;
    final w = s * 0.12;
    final h = s * 0.2;
    final cx = center.dx;
    final cy = center.dy;
    canvas.drawRRect(
      RRect.fromLTRBR(cx - w, cy - h * 1.1, cx + w, cy + h * 0.5, Radius.circular(w)),
      ink,
    );
    canvas.drawArc(
      Rect.fromLTRB(cx - w * 1.8, cy - h * 0.6, cx + w * 1.8, cy + h * 0.9),
      20 * math.pi / 180,
      140 * math.pi / 180,
      false,
      ink,
    );
    canvas.drawLine(Offset(cx, cy + h * 0.9), Offset(cx, cy + h * 1.4), ink);
  }

  @override
  bool shouldRepaint(_BubblePainter oldDelegate) => false;
}

/// Another app's icon, or a neutral placeholder while it loads or when the app is gone.
class AppIcon extends StatelessWidget {
  const AppIcon(this.package, {super.key, this.size = 20});

  final String package;
  final double size;

  @override
  Widget build(BuildContext context) {
    final system = ServicesScope.of(context).system;
    return ListenableBuilder(
      listenable: system,
      builder: (context, _) {
        final icon = system.app(package)?.icon;
        if (icon != null) {
          return Image.memory(icon, width: size, height: size, gaplessPlayback: true);
        }
        return Container(
          width: size,
          height: size,
          decoration: BoxDecoration(
            color: Palette.raised,
            borderRadius: BorderRadius.circular(size / 4),
          ),
        );
      },
    );
  }
}

/// A rating from 1 to 5 as a row of short bars.
class Meter extends StatelessWidget {
  const Meter({super.key, required this.label, required this.value});

  final String label;
  final int value;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(label, style: const TextStyle(fontSize: 12, color: Palette.muted)),
        const SizedBox(width: 6),
        for (var i = 1; i <= 5; i++)
          Container(
            width: 10,
            height: 4,
            margin: const EdgeInsets.only(right: 2),
            decoration: BoxDecoration(
              color: i <= value ? Palette.ember : Palette.raised,
              borderRadius: BorderRadius.circular(2),
            ),
          ),
      ],
    );
  }
}

/// A small rounded label, such as "Not inserted" on a history item.
class Tag extends StatelessWidget {
  const Tag(this.text, {super.key, this.color = Palette.ember});

  final String text;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Text(
        text,
        style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: color),
      ),
    );
  }
}
