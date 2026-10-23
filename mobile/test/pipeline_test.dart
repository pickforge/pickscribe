import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/dictionary.dart';
import 'package:pickscribe/history.dart';
import 'package:pickscribe/models.dart';
import 'package:pickscribe/pipeline.dart';
import 'package:pickscribe/settings.dart';
import 'package:pickscribe/whisper.dart';

/// Answers each call with the next text and records what it was asked.
class FakeModel implements Transcriber {
  FakeModel(this.texts);

  final List<String> texts;
  final calls = <({int samples, List<String> languages, String prompt})>[];
  Completer<void>? gate;
  bool aborted = false;

  @override
  Future<Transcription> transcribe(
    Float32List pcm, {
    required List<String> languages,
    String prompt = '',
    int threads = 6,
    int beamSize = 1,
  }) async {
    calls.add((samples: pcm.length, languages: languages, prompt: prompt));
    await gate?.future;
    if (aborted) throw const WhisperCancelled();
    return Transcription(texts[calls.length - 1], 'pt');
  }

  @override
  void abort() => aborted = true;

  @override
  void clearAbort() => aborted = false;

  @override
  void dispose() {}
}

Float32List tone(double seconds, {bool loud = true}) {
  final random = Random(1);
  return Float32List.fromList([
    for (var i = 0; i < seconds * sampleRate; i++)
      (loud ? 0.14 * sin(2 * pi * 220 * i / sampleRate) : 0) + (random.nextDouble() - 0.5) * 0.002,
  ]);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late Directory dir;
  late HistoryStore history;
  late FakeModel model;
  late Pipeline pipeline;
  var loads = 0;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('pipeline');
    final settings = SettingsStore(dir);
    await settings.load();
    final models = ModelManager(dir);
    await models.fileOf(recommendedModel).writeAsString('model');
    await settings.update(settings.settings.copyWith(model: recommendedModel.file));
    final dictionary = DictionaryStore(dir);
    await dictionary.load();
    history = HistoryStore(dir);
    await history.load();
    loads = 0;
    pipeline = Pipeline(
      models: models,
      settings: settings,
      dictionary: dictionary,
      history: history,
      loadModel: (_) async {
        loads++;
        return model;
      },
    );
  });
  tearDown(() => dir.delete(recursive: true));

  // Feeds [audio] in 200 ms pieces, like the bubble service.
  void stream(int session, Float32List audio) {
    for (var start = 0; start < audio.length; start += 3200) {
      pipeline.audio(
        session,
        Float32List.sublistView(audio, start, min(start + 3200, audio.length)),
      );
    }
  }

  test('a short dictation is one part, and the model loads while recording', () async {
    model = FakeModel(['Olá mundo.']);
    pipeline.begin(1);
    stream(1, tone(3));
    await pumpEventQueue();
    expect(loads, 1);
    final result = await pipeline.finish(1, tone(0.2));
    expect(result['text'], 'Olá mundo.');
    expect(model.calls, hasLength(1));
    expect(model.calls.single.languages, ['pt', 'en']);
    expect(history.items.single.text, 'Olá mundo.');
  });

  test('a long dictation is transcribed in parts while recording', () async {
    model = FakeModel(['Primeira parte.', 'segunda parte.']);
    pipeline.begin(2);
    stream(2, Float32List.fromList([...tone(7), ...tone(1, loud: false)]));
    await pumpEventQueue();
    // The first part went out at the pause, before the user stopped.
    expect(model.calls, hasLength(1));
    final result = await pipeline.finish(2, tone(3));
    expect(result['text'], 'Primeira parte. segunda parte.');
    // The second part picks its own language and continues the first part's text.
    expect(model.calls[1].languages, ['pt', 'en']);
    expect(model.calls[1].prompt, contains('Primeira parte.'));
    expect(history.items, hasLength(1));
  });

  test('a silent end after speech is not transcribed', () async {
    model = FakeModel(['Só isso.']);
    pipeline.begin(3);
    stream(3, Float32List.fromList([...tone(7), ...tone(1, loud: false)]));
    final result = await pipeline.finish(3, tone(3, loud: false));
    expect(result['text'], 'Só isso.');
    expect(model.calls, hasLength(1));
  });

  test('silence before speech is not transcribed', () async {
    model = FakeModel(['Agora sim.']);
    pipeline.begin(8);
    stream(8, Float32List.fromList([...tone(20, loud: false), ...tone(3)]));
    final result = await pipeline.finish(8, tone(0.2));
    expect(result['text'], 'Agora sim.');
    expect(model.calls.single.samples, lessThan(4 * sampleRate));
  });

  test('cancel drops the recording and the part in progress', () async {
    model = FakeModel(['Primeira parte.', 'De novo.'])..gate = Completer();
    pipeline.begin(4);
    stream(4, Float32List.fromList([...tone(7), ...tone(1, loud: false)]));
    await pumpEventQueue();
    pipeline.cancel();
    model.gate!.complete();
    await expectLater(
      pipeline.finish(4, tone(1)),
      throwsA(isA<PlatformException>().having((error) => error.code, 'code', 'cancelled')),
    );
    expect(history.items, isEmpty);

    // The next dictation works.
    pipeline.begin(5);
    stream(5, tone(2));
    expect((await pipeline.finish(5, tone(0.2)))['text'], 'De novo.');
  });

  test('audio for another session is ignored', () async {
    model = FakeModel(['Certo.']);
    pipeline.begin(6);
    stream(7, tone(9));
    stream(6, tone(2));
    expect((await pipeline.finish(6, tone(0.2)))['text'], 'Certo.');
    expect(model.calls.single.samples, lessThan(3 * sampleRate));
  });
}
