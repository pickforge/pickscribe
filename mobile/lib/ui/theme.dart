import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// Pickforge Studio colors: a dark canvas, off-white text, and one ember accent.
abstract final class Palette {
  static const canvas = Color(0xFF0A0A0B);
  static const surface = Color(0xFF141417);
  static const raised = Color(0xFF1C1C20);
  static const border = Color(0x14F2F2F3);
  static const text = Color(0xFFF2F2F3);
  static const muted = Color(0xFF9A9AA0);
  static const faint = Color(0xFF6E6E75);
  static const ember = Color(0xFFFF7A1A);
  static const emberDark = Color(0xFFC2410C);
  static const emberWash = Color(0x24FF7A1A);
  static const danger = Color(0xFFFF6B6B);
}

const sans = 'Geist';
const mono = 'GeistMono';

const _scheme = ColorScheme.dark(
  primary: Palette.ember,
  onPrimary: Palette.canvas,
  primaryContainer: Palette.emberWash,
  onPrimaryContainer: Palette.ember,
  secondary: Palette.text,
  onSecondary: Palette.canvas,
  surface: Palette.canvas,
  onSurface: Palette.text,
  onSurfaceVariant: Palette.muted,
  surfaceContainerLowest: Palette.canvas,
  surfaceContainerLow: Palette.surface,
  surfaceContainer: Palette.surface,
  surfaceContainerHigh: Palette.raised,
  surfaceContainerHighest: Palette.raised,
  outline: Palette.faint,
  outlineVariant: Palette.border,
  error: Palette.danger,
  onError: Palette.canvas,
);

const _text = TextTheme(
  headlineMedium: TextStyle(
    fontSize: 30,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.8,
    height: 1.15,
    color: Palette.text,
  ),
  headlineSmall: TextStyle(
    fontSize: 24,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.5,
    color: Palette.text,
  ),
  titleLarge: TextStyle(
    fontSize: 20,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.3,
    color: Palette.text,
  ),
  titleMedium: TextStyle(
    fontSize: 16,
    fontWeight: FontWeight.w500,
    letterSpacing: -0.1,
    color: Palette.text,
  ),
  titleSmall: TextStyle(fontSize: 14, fontWeight: FontWeight.w500, color: Palette.text),
  bodyLarge: TextStyle(fontSize: 16, height: 1.45, color: Palette.text),
  bodyMedium: TextStyle(fontSize: 14, height: 1.4, color: Palette.text),
  bodySmall: TextStyle(fontSize: 12.5, height: 1.35, color: Palette.muted),
  labelLarge: TextStyle(fontSize: 14, fontWeight: FontWeight.w500, color: Palette.text),
  labelMedium: TextStyle(fontSize: 12, fontWeight: FontWeight.w500, color: Palette.text),
  // Section and day headers.
  labelSmall: TextStyle(fontFamily: mono, fontSize: 11, letterSpacing: 1.2, color: Palette.muted),
);

const _listTile = ListTileThemeData(
  contentPadding: EdgeInsets.symmetric(horizontal: 16),
  iconColor: Palette.muted,
  titleTextStyle: TextStyle(
    fontFamily: sans,
    fontSize: 15,
    fontWeight: FontWeight.w500,
    color: Palette.text,
  ),
  subtitleTextStyle: TextStyle(fontFamily: sans, fontSize: 13, color: Palette.muted),
  leadingAndTrailingTextStyle: TextStyle(fontFamily: sans, fontSize: 14, color: Palette.muted),
);

final _shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(14));

ThemeData buildTheme() {
  return ThemeData(
    useMaterial3: true,
    brightness: Brightness.dark,
    colorScheme: _scheme,
    fontFamily: sans,
    textTheme: _text,
    scaffoldBackgroundColor: Palette.canvas,
    splashFactory: InkSparkle.splashFactory,
    appBarTheme: const AppBarTheme(
      backgroundColor: Palette.canvas,
      surfaceTintColor: Colors.transparent,
      scrolledUnderElevation: 0,
      centerTitle: false,
      titleTextStyle: TextStyle(
        fontFamily: sans,
        fontSize: 18,
        fontWeight: FontWeight.w600,
        color: Palette.text,
      ),
      systemOverlayStyle: SystemUiOverlayStyle(
        statusBarColor: Colors.transparent,
        statusBarIconBrightness: Brightness.light,
        systemNavigationBarColor: Palette.canvas,
      ),
    ),
    navigationBarTheme: _navigationBar(),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(0, 48),
        padding: const EdgeInsets.symmetric(horizontal: 20),
        shape: _shape,
        textStyle: const TextStyle(fontFamily: sans, fontSize: 15, fontWeight: FontWeight.w600),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: Palette.text,
        minimumSize: const Size(0, 48),
        padding: const EdgeInsets.symmetric(horizontal: 20),
        side: const BorderSide(color: Palette.border),
        shape: _shape,
        textStyle: const TextStyle(fontFamily: sans, fontSize: 15, fontWeight: FontWeight.w500),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: Palette.ember,
        shape: _shape,
        textStyle: const TextStyle(fontFamily: sans, fontSize: 14, fontWeight: FontWeight.w600),
      ),
    ),
    sliderTheme: SliderThemeData(
      activeTrackColor: Palette.ember,
      inactiveTrackColor: Palette.raised,
      thumbColor: Palette.ember,
      overlayColor: Palette.emberWash,
      trackHeight: 4,
      tickMarkShape: SliderTickMarkShape.noTickMark,
    ),
    switchTheme: _switch(),
    inputDecorationTheme: _input(),
    bottomSheetTheme: const BottomSheetThemeData(
      backgroundColor: Palette.surface,
      surfaceTintColor: Colors.transparent,
      showDragHandle: true,
      dragHandleColor: Palette.faint,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(28))),
    ),
    dialogTheme: const DialogThemeData(
      backgroundColor: Palette.surface,
      surfaceTintColor: Colors.transparent,
    ),
    snackBarTheme: const SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      backgroundColor: Palette.raised,
      contentTextStyle: TextStyle(fontFamily: sans, color: Palette.text, fontSize: 14),
      actionTextColor: Palette.ember,
    ),
    listTileTheme: _listTile,
    floatingActionButtonTheme: _floatingButton(),
    dividerTheme: const DividerThemeData(color: Palette.border, thickness: 1, space: 1),
    progressIndicatorTheme: const ProgressIndicatorThemeData(
      color: Palette.ember,
      linearTrackColor: Palette.raised,
    ),
    chipTheme: ChipThemeData(
      backgroundColor: Palette.raised,
      selectedColor: Palette.emberWash,
      side: const BorderSide(color: Palette.border),
      labelStyle: const TextStyle(fontFamily: sans, fontSize: 14, color: Palette.text),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
      checkmarkColor: Palette.ember,
    ),
  );
}

NavigationBarThemeData _navigationBar() => NavigationBarThemeData(
  backgroundColor: Palette.canvas,
  surfaceTintColor: Colors.transparent,
  indicatorColor: Palette.emberWash,
  height: 68,
  labelTextStyle: WidgetStateProperty.resolveWith(
    (states) => TextStyle(
      fontFamily: sans,
      fontSize: 12,
      fontWeight: FontWeight.w500,
      color: states.contains(WidgetState.selected) ? Palette.text : Palette.faint,
    ),
  ),
  iconTheme: WidgetStateProperty.resolveWith(
    (states) => IconThemeData(
      size: 22,
      color: states.contains(WidgetState.selected) ? Palette.ember : Palette.faint,
    ),
  ),
);

SwitchThemeData _switch() => SwitchThemeData(
  thumbColor: WidgetStateProperty.resolveWith(
    (states) => states.contains(WidgetState.selected) ? Palette.canvas : Palette.muted,
  ),
  trackColor: WidgetStateProperty.resolveWith(
    (states) => states.contains(WidgetState.selected) ? Palette.ember : Palette.raised,
  ),
  trackOutlineColor: const WidgetStatePropertyAll(Colors.transparent),
);

InputDecorationTheme _input() => InputDecorationTheme(
  filled: true,
  fillColor: Palette.raised,
  contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
  border: OutlineInputBorder(borderRadius: BorderRadius.circular(14), borderSide: BorderSide.none),
  focusedBorder: OutlineInputBorder(
    borderRadius: BorderRadius.circular(14),
    borderSide: const BorderSide(color: Palette.ember, width: 1.5),
  ),
  errorBorder: OutlineInputBorder(
    borderRadius: BorderRadius.circular(14),
    borderSide: const BorderSide(color: Palette.danger),
  ),
  labelStyle: const TextStyle(color: Palette.muted),
  floatingLabelStyle: const TextStyle(color: Palette.ember),
  hintStyle: const TextStyle(color: Palette.faint),
);

FloatingActionButtonThemeData _floatingButton() => FloatingActionButtonThemeData(
  backgroundColor: Palette.ember,
  foregroundColor: Palette.canvas,
  elevation: 0,
  focusElevation: 0,
  hoverElevation: 0,
  highlightElevation: 0,
  extendedTextStyle: const TextStyle(fontFamily: sans, fontSize: 15, fontWeight: FontWeight.w600),
  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(18)),
);
