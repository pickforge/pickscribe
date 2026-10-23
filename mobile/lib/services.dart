import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:path_provider/path_provider.dart';

import 'bridge.dart';
import 'dictionary.dart';
import 'history.dart';
import 'models.dart';
import 'pipeline.dart';
import 'settings.dart';

/// Everything the screens and the bubble pipeline share. The engine starts with the app
/// process, often with no screen open, so this loads before any widget.
class Services {
  Services._(Directory dir)
    : _appFile = File('${dir.path}/app.json'),
      system = SystemState(const SystemBridge()),
      settings = SettingsStore(dir),
      dictionary = DictionaryStore(dir),
      history = HistoryStore(dir),
      models = ModelManager(dir) {
    pipeline = Pipeline(
      models: models,
      settings: settings,
      dictionary: dictionary,
      history: history,
    );
  }

  final File _appFile;
  final SystemState system;
  final SettingsStore settings;
  final DictionaryStore dictionary;
  final HistoryStore history;
  final ModelManager models;
  late final Pipeline pipeline;

  /// Whether the user finished or skipped the setup steps once.
  final onboarded = ValueNotifier(false);

  /// Loads everything from the app's private storage, or from [dir] in tests.
  static Future<Services> start({Directory? dir}) async {
    final services = Services._(dir ?? await getApplicationSupportDirectory());
    await Future.wait([
      services.settings.load(),
      services.dictionary.load(),
      services.history.load(),
      services._loadAppState(),
    ]);
    services.pipeline.listen();
    return services;
  }

  Future<void> _loadAppState() async {
    try {
      if (await _appFile.exists()) {
        final json = jsonDecode(await _appFile.readAsString()) as Map<String, Object?>;
        onboarded.value = json['onboarded'] == true;
      }
    } on FormatException {
      onboarded.value = false;
    }
  }

  Future<void> finishOnboarding() async {
    onboarded.value = true;
    await _appFile.writeAsString(jsonEncode({'onboarded': true}), flush: true);
  }
}

class ServicesScope extends InheritedWidget {
  const ServicesScope({super.key, required this.services, required super.child});

  final Services services;

  static Services of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<ServicesScope>()!.services;

  @override
  bool updateShouldNotify(ServicesScope oldWidget) => services != oldWidget.services;
}
