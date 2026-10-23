import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'dictionary.dart';
import 'history.dart';
import 'models.dart';
import 'settings.dart';
import 'whisper.dart';

const pipelineChannel = MethodChannel('pickscribe/pipeline');
const sampleRate = 16000;

/// Turns a recording from the bubble into text and keeps it in History. It runs on the app's
/// single engine, so the model stays loaded between dictations.
class Pipeline {
  Pipeline({
    required this.models,
    required this.settings,
    required this.dictionary,
    required this.history,
  });

  final ModelManager models;
  final SettingsStore settings;
  final DictionaryStore dictionary;
  final HistoryStore history;

  Whisper? _whisper;
  String? _loadedPath;
  Future<void> _queue = Future.value();
  int _jobs = 0;
  int _cancelledThrough = 0;

  void listen() => pipelineChannel.setMethodCallHandler((call) async {
    final args = call.arguments as Map?;
    switch (call.method) {
      case 'process':
        return process(args!['pcm'] as Float32List, app: args['app'] as String?);
      case 'cancel':
        cancel();
        return null;
      case 'delivered':
        await history.markDelivered(args!['id'] as String, args['ok'] as bool);
        return null;
      default:
        throw MissingPluginException(call.method);
    }
  });

  /// Transcribes [pcm], applies the dictionary, and adds the text to History. Returns the
  /// History id and the text. Dictations run one at a time, in order.
  Future<Map<String, String>> process(Float32List pcm, {String? app}) {
    final job = ++_jobs;
    final result = _queue.then((_) => _run(job, pcm, app));
    _queue = result.then((_) {}, onError: (_) {});
    return result;
  }

  /// Drops every dictation that started before now.
  void cancel() {
    _cancelledThrough = _jobs;
    _whisper?.abort();
  }

  Future<Map<String, String>> _run(int job, Float32List pcm, String? app) async {
    if (job <= _cancelledThrough) throw _cancelled();
    final config = settings.settings;
    final model = modelByFile(config.model);
    if (model == null || !models.isDownloaded(model)) {
      throw PlatformException(code: 'model', message: 'Open PickScribe and download a model.');
    }
    final clock = Stopwatch()..start();
    final whisper = await _load(models.fileOf(model).path);
    final loadMs = clock.elapsedMilliseconds;
    if (job <= _cancelledThrough) throw _cancelled();
    whisper.clearAbort();
    final Transcription transcription;
    try {
      transcription = await whisper.transcribe(
        pcm,
        languages: config.languages,
        prompt: dictionary.dictionary.prompt,
        threads: config.threads,
        beamSize: config.beamSize,
      );
    } on WhisperCancelled {
      throw _cancelled();
    } on WhisperException catch (error) {
      throw PlatformException(code: 'whisper', message: error.message);
    }
    final audioMs = pcm.length * 1000 ~/ sampleRate;
    // Timings only: logs never hold dictated text.
    debugPrint(
      'PickScribe: ${model.file}, ${audioMs}ms audio, load ${loadMs}ms, '
      'transcribe ${clock.elapsedMilliseconds - loadMs}ms, ${transcription.language}',
    );
    final text = dictionary.dictionary.apply(transcription.text);
    if (text.trim().isEmpty) return {'text': ''};
    final now = DateTime.now();
    final item = HistoryItem(
      id: now.microsecondsSinceEpoch.toString(),
      at: now,
      text: text,
      transcript: transcription.text,
      audioMs: audioMs,
      app: app,
    );
    try {
      await history.add(item);
    } on FileSystemException {
      // A full disk must not cost the dictation, so the text still goes to the field.
      return {'text': text};
    }
    return {'id': item.id, 'text': text};
  }

  Future<Whisper> _load(String path) async {
    if (_loadedPath != path) {
      _whisper?.dispose();
      _whisper = null;
      _loadedPath = null;
      _whisper = await Whisper.load(path);
      _loadedPath = path;
    }
    return _whisper!;
  }

  static PlatformException _cancelled() =>
      PlatformException(code: 'cancelled', message: 'Cancelled');
}
