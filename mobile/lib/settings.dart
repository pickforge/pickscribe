import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'storage.dart';

/// Languages offered for dictation, by Whisper code.
const languageNames = {
  'pt': 'Portuguese',
  'en': 'English',
  'es': 'Spanish',
  'fr': 'French',
  'de': 'German',
  'it': 'Italian',
};

/// How dictation transcribes, stored in `transcription.json` in the app's private storage.
@immutable
class TranscriptionSettings {
  const TranscriptionSettings({
    this.model,
    this.languages = const ['pt', 'en'],
    this.threads = 6,
    this.beamSize = 1,
  });

  /// The model file name, or null before one is chosen.
  final String? model;

  /// Whisper picks the most likely of these for each dictation.
  final List<String> languages;
  final int threads;

  /// 1 decodes greedily, which is much faster on the phone than a beam search.
  final int beamSize;

  TranscriptionSettings copyWith({
    String? model,
    List<String>? languages,
    int? threads,
    int? beamSize,
  }) => TranscriptionSettings(
    model: model ?? this.model,
    languages: languages ?? this.languages,
    threads: threads ?? this.threads,
    beamSize: beamSize ?? this.beamSize,
  );

  Map<String, Object?> toJson() => {
    'model': ?model,
    'languages': languages,
    'threads': threads,
    'beam_size': beamSize,
  };

  factory TranscriptionSettings.fromJson(Map<String, Object?> json) {
    const defaults = TranscriptionSettings();
    final languages = (json['languages'] as List?)
        ?.whereType<String>()
        .where(languageNames.containsKey)
        .toList();
    return TranscriptionSettings(
      model: json['model'] as String?,
      languages: languages == null || languages.isEmpty ? defaults.languages : languages,
      threads: (json['threads'] as int? ?? defaults.threads).clamp(1, 8),
      beamSize: (json['beam_size'] as int? ?? defaults.beamSize).clamp(1, 8),
    );
  }
}

class SettingsStore extends ChangeNotifier {
  SettingsStore(Directory dir)
    : _file = SafeFile(File('${dir.path}/transcription.json')),
      _legacy = File('${dir.path}/dictation.json');

  final SafeFile _file;

  // The first prototype saved only the model, with slower decoding, in this file.
  final File _legacy;
  TranscriptionSettings _settings = const TranscriptionSettings();

  TranscriptionSettings get settings => _settings;

  Future<void> load() async {
    try {
      if (await _file.file.exists()) {
        _settings = TranscriptionSettings.fromJson(
          jsonDecode(await _file.file.readAsString()) as Map<String, Object?>,
        );
      } else if (await _legacy.exists()) {
        final legacy = jsonDecode(await _legacy.readAsString()) as Map<String, Object?>;
        _settings = TranscriptionSettings(model: legacy['model'] as String?);
      }
    } on FormatException {
      _settings = const TranscriptionSettings();
    }
    notifyListeners();
  }

  Future<void> update(TranscriptionSettings settings) async {
    _settings = settings;
    notifyListeners();
    await _file.write(jsonEncode(settings.toJson()));
  }
}
