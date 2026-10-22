import 'dart:io';

import 'package:crypto/crypto.dart';

/// A multilingual Whisper model from the whisper.cpp repository on Hugging Face.
class WhisperModel {
  const WhisperModel(this.file, this.bytes, this.sha256);

  final String file;
  final int bytes;
  final String sha256;

  Uri get url =>
      Uri.parse('https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$file');

  String get label => '$file (${(bytes / 1e6).round()} MB)';
}

const models = [
  WhisperModel(
    'ggml-base.bin',
    147951465,
    '60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe',
  ),
  WhisperModel(
    'ggml-small-q5_1.bin',
    190085487,
    'ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb',
  ),
  WhisperModel(
    'ggml-medium-q5_0.bin',
    539212467,
    '19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f',
  ),
  WhisperModel(
    'ggml-large-v3-turbo-q5_0.bin',
    574041195,
    '394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2',
  ),
  WhisperModel(
    'ggml-large-v3-turbo-q8_0.bin',
    874188075,
    '317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1',
  ),
];

/// Downloads [model] into [dir] and checks its SHA-256 before the file takes its final
/// name, so a partial or tampered download is never loaded.
Future<File> download(
  WhisperModel model,
  Directory dir, {
  void Function(int received)? onProgress,
}) async {
  final target = File('${dir.path}/${model.file}');
  if (target.existsSync()) return target;
  final part = File('${target.path}.part');
  final client = HttpClient();
  try {
    final response = await (await client.getUrl(model.url)).close();
    if (response.statusCode != HttpStatus.ok) {
      throw HttpException('HTTP ${response.statusCode}', uri: model.url);
    }
    final sink = part.openWrite();
    final digest = _DigestSink();
    final hasher = sha256.startChunkedConversion(digest);
    var received = 0;
    await for (final chunk in response) {
      sink.add(chunk);
      hasher.add(chunk);
      received += chunk.length;
      onProgress?.call(received);
    }
    await sink.close();
    hasher.close();
    if (digest.value.toString() != model.sha256) {
      part.deleteSync();
      throw StateError('${model.file} failed its SHA-256 check');
    }
    return part.renameSync(target.path);
  } finally {
    client.close();
  }
}

class _DigestSink implements Sink<Digest> {
  late Digest value;

  @override
  void add(Digest data) => value = data;

  @override
  void close() {}
}
