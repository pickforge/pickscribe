import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The Kotlin side of the app: permissions, the bubble settings, and other apps' names.
/// Every channel call the screens make goes through here.
class SystemBridge {
  const SystemBridge();

  static const _system = MethodChannel('pickscribe/system');
  static const _bubble = MethodChannel('pickscribe/bubble');

  Future<SetupStatus> status() async {
    final map = await _system.invokeMapMethod<String, Object?>('status') ?? const {};
    return SetupStatus(
      microphone: map['microphone'] == true,
      service: map['service'] == true,
      version: map['version'] as String? ?? '',
    );
  }

  Future<void> requestMicrophone() => _system.invokeMethod<void>('requestMicrophone');

  Future<void> openAccessibilitySettings() =>
      _system.invokeMethod<void>('openAccessibilitySettings');

  Future<List<AppInfo>> describeApps(List<String> packages, {int iconPx = 96}) async {
    final list = await _system.invokeListMethod<Map<Object?, Object?>>('describeApps', {
      'packages': packages,
      'iconPx': iconPx,
    });
    return [
      for (final map in list ?? const <Map<Object?, Object?>>[])
        AppInfo(
          map['package'] as String,
          label: map['label'] as String?,
          icon: map['icon'] as Uint8List?,
        ),
    ];
  }

  Future<List<AppInfo>> launchableApps() async {
    final list = await _system.invokeListMethod<Map<Object?, Object?>>('launchableApps');
    return [
      for (final map in list ?? const <Map<Object?, Object?>>[])
        AppInfo(map['package'] as String, label: map['label'] as String?),
    ];
  }

  Future<BubbleLook> bubble([String method = 'get', Map<String, Object?>? args]) async {
    final map = await _bubble.invokeMapMethod<String, Object?>(method, args) ?? const {};
    final until = map['snoozedUntil'] as int? ?? 0;
    return BubbleLook(
      size: map['size'] as int? ?? 56,
      opacity: (map['opacity'] as num?)?.toDouble() ?? 0.9,
      blocked: (map['blocked'] as List?)?.cast<String>() ?? const [],
      snoozedUntil: until > 0 ? DateTime.fromMillisecondsSinceEpoch(until) : null,
    );
  }
}

@immutable
class SetupStatus {
  const SetupStatus({required this.microphone, required this.service, this.version = ''});

  final bool microphone;

  /// Whether the accessibility service that draws the bubble is on.
  final bool service;

  /// The app's version name.
  final String version;
}

@immutable
class AppInfo {
  const AppInfo(this.package, {this.label, this.icon});

  final String package;

  /// Null when the app is not installed.
  final String? label;
  final Uint8List? icon;

  String get name => label ?? package;
}

@immutable
class BubbleLook {
  const BubbleLook({
    required this.size,
    required this.opacity,
    required this.blocked,
    required this.snoozedUntil,
  });

  static const minSize = 40;
  static const maxSize = 80;
  static const minOpacity = 0.3;

  final int size;
  final double opacity;
  final List<String> blocked;
  final DateTime? snoozedUntil;

  bool snoozedAt(DateTime now) => snoozedUntil != null && snoozedUntil!.isAfter(now);
}

/// Setup state, bubble settings, and app names, kept fresh for the screens.
class SystemState extends ChangeNotifier {
  SystemState(this.bridge);

  final SystemBridge bridge;

  /// False until the first [refresh] reads the real values.
  bool loaded = false;
  SetupStatus status = const SetupStatus(microphone: false, service: false);
  BubbleLook bubble = const BubbleLook(size: 56, opacity: 0.9, blocked: [], snoozedUntil: null);
  final _apps = <String, AppInfo>{};
  final _pending = <String>{};

  /// Reads permissions and bubble settings again, such as after the user returns from the
  /// system settings.
  Future<void> refresh() async {
    try {
      status = await bridge.status();
      bubble = await bridge.bubble();
    } on MissingPluginException {
      // The Kotlin handlers live with the screen, which may not be open yet.
      return;
    }
    loaded = true;
    notifyListeners();
  }

  Future<void> setLook({int? size, double? opacity}) async {
    bubble = await bridge.bubble('setLook', {'size': ?size, 'opacity': ?opacity});
    notifyListeners();
  }

  Future<void> resume() async {
    bubble = await bridge.bubble('resume');
    notifyListeners();
  }

  Future<void> block(String package) async {
    bubble = await bridge.bubble('block', {'package': package});
    notifyListeners();
  }

  Future<void> unblock(String package) async {
    bubble = await bridge.bubble('unblock', {'package': package});
    notifyListeners();
  }

  /// The app's name and icon, or null until they load. Asking starts the load.
  AppInfo? app(String package) {
    final known = _apps[package];
    if (known != null || _pending.contains(package)) return known;
    _pending.add(package);
    // Batch the names the current frame asks for into one call.
    if (_pending.length == 1) Future.microtask(_loadPending);
    return null;
  }

  Future<void> _loadPending() async {
    final packages = _pending.toList();
    try {
      for (final info in await bridge.describeApps(packages)) {
        _apps[info.package] = info;
      }
    } on PlatformException {
      for (final package in packages) {
        _apps[package] = AppInfo(package);
      }
    } finally {
      _pending.removeAll(packages);
    }
    notifyListeners();
  }
}
