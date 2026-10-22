import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

import 'whisper.dart';

const pipelineChannel = MethodChannel('pickscribe/pipeline');

/// Which model and decoding settings dictation uses. Stored as JSON next to the models.
class DictationConfig {
  const DictationConfig({required this.model, this.beamSize = 5, this.threads = 6});

  final String model;
  final int beamSize;
  final int threads;

  static Future<File> _file() async =>
      File('${(await getApplicationSupportDirectory()).path}/dictation.json');

  static Future<DictationConfig?> load() async {
    final file = await _file();
    if (!file.existsSync()) return null;
    final json = jsonDecode(await file.readAsString()) as Map<String, dynamic>;
    return DictationConfig(
      model: json['model'] as String,
      beamSize: json['beam_size'] as int,
      threads: json['threads'] as int,
    );
  }

  Future<void> save() async => (await _file()).writeAsString(
    jsonEncode({'model': model, 'beam_size': beamSize, 'threads': threads}),
  );
}

/// Turns a recording from the bubble into text. It runs on the app's single engine, so the
/// model stays loaded between dictations.
class Pipeline {
  Whisper? _whisper;
  String? _loadedPath;

  void listen() => pipelineChannel.setMethodCallHandler((call) async {
    if (call.method != 'process') throw MissingPluginException(call.method);
    final args = call.arguments as Map;
    return process(args['pcm'] as Float32List);
  });

  Future<String> process(Float32List pcm) async {
    final config = await DictationConfig.load();
    if (config == null) {
      throw PlatformException(code: 'model', message: 'Choose a model in PickScribe first.');
    }
    final dir = await getApplicationSupportDirectory();
    final path = '${dir.path}/models/${config.model}';
    if (_loadedPath != path) {
      _whisper?.dispose();
      _whisper = await Whisper.load(path);
      _loadedPath = path;
    }
    final result = await _whisper!.transcribe(
      pcm,
      languages: const ['pt', 'en'],
      threads: config.threads,
      beamSize: config.beamSize,
    );
    return result.text;
  }
}

