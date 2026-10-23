import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'dictionary.dart';
import 'history.dart';
import 'models.dart';
import 'pauses.dart';
import 'settings.dart';
import 'whisper.dart';

const pipelineChannel = MethodChannel('pickscribe/pipeline');
const sampleRate = 16000;

typedef ModelLoader = Future<Transcriber> Function(String path);

/// Turns a recording from the bubble into text and keeps it in History. The audio arrives
/// while the user speaks, and each part that ends at a pause is transcribed right away, so
/// only the last part is left when the user stops. It runs on the app's single engine, so
/// the model stays loaded between dictations.
class Pipeline {
  Pipeline({
    required this.models,
    required this.settings,
    required this.dictionary,
    required this.history,
    ModelLoader? loadModel,
  }) : _loadModel = loadModel ?? Whisper.load;

  final ModelManager models;
  final SettingsStore settings;
  final DictionaryStore dictionary;
  final HistoryStore history;
  final ModelLoader _loadModel;

  Transcriber? _model;
  String? _loadedPath;
  _Recording? _recording;
  Future<void> _queue = Future.value();
  int _jobs = 0;
  int _cancelledThrough = 0;

  void listen() => pipelineChannel.setMethodCallHandler((call) async {
    final args = call.arguments as Map?;
    switch (call.method) {
      case 'begin':
        begin(args!['session'] as int);
        return null;
      case 'audio':
        audio(args!['session'] as int, args['pcm'] as Float32List);
        return null;
      case 'finish':
        return finish(
          args!['session'] as int,
          args['pcm'] as Float32List,
          app: args['app'] as String?,
        );
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

  /// Starts recording [session] and loads the model while the user speaks.
  void begin(int session) {
    _recording = _Recording(session);
    _enqueue((job) async {
      final model = _currentModel();
      if (model != null) await _load(models.fileOf(model).path);
    }).ignore();
  }

  /// Adds audio to [session] and transcribes the part before a pause, if one ended.
  void audio(int session, Float32List pcm) {
    final recording = _recording;
    if (recording == null || recording.session != session) return;
    recording.add(pcm);
    final cut = recording.levels.cut(recording.partStart);
    if (cut != null) _addPart(recording, cut);
  }

  /// Adds the last audio of [session], waits for every part, applies the dictionary, and adds
  /// the text to History. Returns the History id and the text.
  Future<Map<String, Object>> finish(int session, Float32List pcm, {String? app}) async {
    final recording = _recording;
    if (recording == null || recording.session != session) throw _cancelled();
    _recording = null;
    recording.add(pcm);
    final clock = Stopwatch()..start();
    _addPart(recording, recording.length, last: true);
    final parts = await Future.wait(recording.parts);
    final transcript = parts.where((part) => part.isNotEmpty).join(' ');
    final audioMs = recording.length * 1000 ~/ sampleRate;
    // Timings only: logs never hold dictated text.
    debugPrint(
      'PickScribe: ${settings.settings.model}, ${audioMs}ms audio in ${parts.length} parts, '
      '${clock.elapsedMilliseconds}ms after stop, ${recording.language}',
    );
    final text = dictionary.dictionary.apply(transcript);
    if (text.trim().isEmpty) return {'text': ''};
    final now = DateTime.now();
    final item = HistoryItem(
      id: now.microsecondsSinceEpoch.toString(),
      at: now,
      text: text,
      transcript: transcript,
      audioMs: audioMs,
      app: app,
    );
    try {
      await history.add(item);
    } on FileSystemException {
      // A full disk must not cost the dictation, so the text still goes to the field. The
      // bubble must not say that History kept it.
      return {'id': item.id, 'text': text, 'saved': false};
    }
    return {'id': item.id, 'text': text};
  }

  /// Drops the recording and every transcription that started before now.
  void cancel() {
    _recording = null;
    _cancelledThrough = _jobs;
    _model?.abort();
  }

  /// Queues the part of [recording] from its last cut to [end]. Silent parts and scraps under
  /// a quarter second are skipped, except a last part that would leave the recording with none.
  void _addPart(_Recording recording, int end, {bool last = false}) {
    final start = recording.partStart;
    recording.partStart = end;
    final speech = recording.levels.speech(start, end);
    final (from, to) = speech ?? (start, end);
    final keep = speech != null && to - from >= sampleRate ~/ 4;
    if (!keep && !(last && recording.parts.isEmpty)) return;
    final pcm = recording.copy(from, to);
    final part = _enqueue((job) => _transcribe(job, recording, pcm));
    // finish reports a failed part; until then, nothing else should.
    part.ignore();
    recording.parts.add(part);
  }

  Future<String> _transcribe(int job, _Recording recording, Float32List pcm) async {
    final config = settings.settings;
    final model = _currentModel();
    if (model == null) {
      throw PlatformException(code: 'model', message: 'Open PickScribe and download a model.');
    }
    final whisper = await _load(models.fileOf(model).path);
    if (job <= _cancelledThrough) throw _cancelled();
    whisper.clearAbort();
    final Transcription transcription;
    try {
      transcription = await whisper.transcribe(
        pcm,
        // The first part picks the language, and later parts keep it.
        languages: recording.language == null ? config.languages : [recording.language!],
        prompt: [
          dictionary.dictionary.prompt,
          recording.context,
        ].where((text) => text.isNotEmpty).join(' '),
        threads: config.threads,
        beamSize: config.beamSize,
      );
    } on WhisperCancelled {
      throw _cancelled();
    } on WhisperException catch (error) {
      throw PlatformException(code: 'whisper', message: error.message);
    }
    final text = transcription.text;
    if (text.isEmpty) return text;
    if (transcription.language != 'auto') recording.language ??= transcription.language;
    // The end of the text so far, so the next part continues the sentence.
    recording.context = text.substring(max(0, text.length - 200));
    return text;
  }

  /// Runs [work] after every earlier job, unless a cancel comes first.
  Future<T> _enqueue<T>(Future<T> Function(int job) work) {
    final job = ++_jobs;
    final result = _queue.then((_) {
      if (job <= _cancelledThrough) throw _cancelled();
      return work(job);
    });
    _queue = result.then((_) {}, onError: (_) {});
    return result;
  }

  WhisperModel? _currentModel() {
    final model = modelByFile(settings.settings.model);
    return model != null && models.isDownloaded(model) ? model : null;
  }

  Future<Transcriber> _load(String path) async {
    if (_loadedPath != path) {
      _model?.dispose();
      _model = null;
      _loadedPath = null;
      _model = await _loadModel(path);
      _loadedPath = path;
    }
    return _model!;
  }

  static PlatformException _cancelled() =>
      PlatformException(code: 'cancelled', message: 'Cancelled');
}

/// One recording from the bubble, with the parts already sent for transcription.
class _Recording {
  _Recording(this.session);

  final int session;
  final levels = SpeechLevels();
  final parts = <Future<String>>[];
  Float32List _audio = Float32List(30 * sampleRate);
  int length = 0;

  /// Where the next part starts.
  int partStart = 0;

  String? language;
  String context = '';

  void add(Float32List pcm) {
    if (length + pcm.length > _audio.length) {
      final grown = Float32List(max(_audio.length * 2, length + pcm.length));
      grown.setRange(0, length, _audio);
      _audio = grown;
    }
    _audio.setRange(length, length + pcm.length, pcm);
    length += pcm.length;
    levels.add(_audio, length);
  }

  Float32List copy(int start, int end) => Float32List.sublistView(_audio, start, end).sublist(0);
}
