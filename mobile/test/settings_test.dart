import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/settings.dart';

void main() {
  late Directory dir;

  setUp(() async => dir = await Directory.systemTemp.createTemp('settings'));
  tearDown(() => dir.delete(recursive: true));

  test('takes only the model from the prototype file, so decoding gets the fast default', () async {
    await File('${dir.path}/dictation.json')
        .writeAsString('{"model":"ggml-base.bin","beam_size":5,"threads":6}');
    final store = SettingsStore(dir);
    await store.load();
    expect(store.settings.model, 'ggml-base.bin');
    expect(store.settings.beamSize, 1);
  });

  test('saves and loads, and drops unknown languages', () async {
    final store = SettingsStore(dir);
    await store.load();
    await store.update(store.settings.copyWith(model: 'ggml-small-q5_1.bin', languages: ['en']));
    final again = SettingsStore(dir);
    await again.load();
    expect(again.settings.model, 'ggml-small-q5_1.bin');
    expect(again.settings.languages, ['en']);

    expect(
      TranscriptionSettings.fromJson({
        'languages': ['xx'],
      }).languages,
      ['pt', 'en'],
    );
  });
}
