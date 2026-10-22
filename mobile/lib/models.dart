import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';

/// A multilingual Whisper model from the whisper.cpp repository on Hugging Face.
@immutable
class WhisperModel {
  const WhisperModel(
    this.file,
    this.name,
    this.bytes,
    this.sha256, {
    required this.speed,
    required this.accuracy,
  });

  final String file;
  final String name;
  final int bytes;
  final String sha256;

  /// Relative ratings from 1 to 5, for the model list.
  final int speed;
  final int accuracy;

  Uri get url => Uri.parse('https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$file');

  String get size =>
      bytes >= 1e9 ? '${(bytes / 1e9).toStringAsFixed(1)} GB' : '${(bytes / 1e6).round()} MB';
}

const models = [
  WhisperModel(
    'ggml-base.bin',
    'Base',
    147951465,
    '60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe',
    speed: 5,
    accuracy: 2,
  ),
  WhisperModel(
    'ggml-small-q5_1.bin',
    'Small',
    190085487,
    'ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb',
    speed: 4,
    accuracy: 3,
  ),
  WhisperModel(
    'ggml-medium-q5_0.bin',
    'Medium',
    539212467,
    '19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f',
    speed: 2,
    accuracy: 4,
  ),
  WhisperModel(
    'ggml-large-v3-turbo-q5_0.bin',
    'Large Turbo',
    574041195,
    '394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2',
    speed: 2,
    accuracy: 5,
  ),
  WhisperModel(
    'ggml-large-v3-turbo-q8_0.bin',
    'Large Turbo, 8-bit',
    874188075,
    '317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1',
    speed: 1,
    accuracy: 5,
  ),
];

/// The model new installs download first.
final recommendedModel = models.firstWhere((model) => model.file == 'ggml-large-v3-turbo-q5_0.bin');

WhisperModel? modelByFile(String? file) {
  for (final model in models) {
    if (model.file == file) return model;
  }
  return null;
}

/// Downloads, checks, and deletes models in the app's private `models` folder. Downloads keep
/// going when the screen closes, because the engine outlives it.
class ModelManager extends ChangeNotifier {
  ModelManager(Directory support) : dir = Directory('${support.path}/models') {
    dir.createSync(recursive: true);
  }

  final Directory dir;
  final _progress = <String, double>{};
  final _clients = <String, HttpClient>{};
  final _errors = <String, String>{};

  File fileOf(WhisperModel model) => File('${dir.path}/${model.file}');

  bool isDownloaded(WhisperModel model) => fileOf(model).existsSync();

  /// From 0 to 1 while downloading, otherwise null.
  double? progressOf(WhisperModel model) => _progress[model.file];

  String? errorOf(WhisperModel model) => _errors[model.file];

  /// Downloads [model] and checks its SHA-256 before the file takes its final name, so a
  /// partial or tampered download is never loaded. Returns false when it failed or stopped.
  Future<bool> download(WhisperModel model) async {
    if (isDownloaded(model) || _progress.containsKey(model.file)) return true;
    final part = File('${fileOf(model).path}.part');
    final client = HttpClient();
    _clients[model.file] = client;
    _errors.remove(model.file);
    _progress[model.file] = 0;
    notifyListeners();
    try {
      final response = await (await client.getUrl(model.url)).close();
      if (response.statusCode != HttpStatus.ok) {
        throw HttpException('HTTP ${response.statusCode}', uri: model.url);
      }
      final sink = part.openWrite();
      final digest = _DigestSink();
      final hasher = sha256.startChunkedConversion(digest);
      var received = 0;
      var shown = 0.0;
      try {
        await for (final chunk in response) {
          sink.add(chunk);
          hasher.add(chunk);
          received += chunk.length;
          final progress = received / model.bytes;
          if (progress - shown >= 0.005) {
            shown = progress;
            _progress[model.file] = progress;
            notifyListeners();
          }
        }
      } finally {
        await sink.close();
      }
      hasher.close();
      if (digest.value.toString() != model.sha256) {
        throw const FileSystemException('The download failed its SHA-256 check');
      }
      await part.rename(fileOf(model).path);
      return true;
    } catch (error) {
      if (_clients.containsKey(model.file)) _errors[model.file] = _describe(error);
      if (await part.exists()) await part.delete();
      return false;
    } finally {
      _clients.remove(model.file)?.close(force: true);
      _progress.remove(model.file);
      notifyListeners();
    }
  }

  void cancel(WhisperModel model) {
    // Removing the client first marks the failure that follows as a cancel, not an error.
    _clients.remove(model.file)?.close(force: true);
  }

  Future<void> delete(WhisperModel model) async {
    final file = fileOf(model);
    if (await file.exists()) await file.delete();
    notifyListeners();
  }

  static String _describe(Object error) => switch (error) {
    SocketException() => 'No connection. Try again on Wi-Fi.',
    HttpException(:final message) => 'The server answered $message.',
    FileSystemException(:final message) => '$message.',
    _ => 'The download failed.',
  };
}

class _DigestSink implements Sink<Digest> {
  late Digest value;

  @override
  void add(Digest data) => value = data;

  @override
  void close() {}
}
