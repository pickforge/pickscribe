import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/pauses.dart';

const rate = 16000;

/// Builds a recording from (seconds, loud) spans: loud spans are a tone near -20 dB, quiet
/// ones a noise floor near -60 dB.
Float32List recording(List<(double, bool)> spans) {
  final random = Random(1);
  final samples = <double>[];
  for (final (seconds, loud) in spans) {
    for (var i = 0; i < seconds * rate; i++) {
      final noise = (random.nextDouble() - 0.5) * 0.002;
      samples.add(loud ? 0.14 * sin(2 * pi * 220 * i / rate) + noise : noise);
    }
  }
  return Float32List.fromList(samples);
}

SpeechLevels measure(Float32List audio) => SpeechLevels()..add(audio, audio.length);

void main() {
  test('cuts in the middle of the first long pause after six seconds', () {
    final audio = recording([
      (0.5, false),
      (4, true),
      (0.8, false),
      (3, true),
      (1, false),
      (2, true),
    ]);
    final cut = measure(audio).cut(0)!;
    // The first pause comes before six seconds. The second runs from 8.3 s to 9.3 s, and the
    // cut lands 0.3 s into it once 0.6 s of it has passed.
    expect(cut / rate, closeTo(8.6, 0.05));
  });

  test('keeps listening through short gaps and before six seconds', () {
    final levels = measure(
      recording([(3, true), (1, false), (1.5, true), (0.3, false), (1.5, true), (0.2, false)]),
    );
    expect(levels.cut(0), isNull);
  });

  test('cuts a part with no pause at its quietest moment after twenty seconds', () {
    final spans = <(double, bool)>[(0.5, false)];
    for (var i = 0; i < 11; i++) {
      spans.addAll([(1.8, true), (0.15, false)]);
    }
    final cut = measure(recording(spans)).cut(0);
    expect(cut, isNotNull);
    expect(cut! / rate, greaterThan(6));
  });

  test('a later part counts from its own start', () {
    final audio = recording([(4, true), (1, false), (4, true), (1, false), (4, true)]);
    final levels = measure(audio);
    final first = levels.cut(0)!;
    expect(levels.cut(first), isNull);
  });

  test('trims silence around speech and finds silent spans', () {
    final audio = recording([(2, false), (3, true), (2, false)]);
    final levels = measure(audio);
    final (from, to) = levels.speech(0, audio.length)!;
    expect(from / rate, closeTo(1.6, 0.05));
    expect(to / rate, closeTo(5.4, 0.05));
    expect(levels.speech(6 * rate, audio.length), isNull);
  });

  test('without a clear pause, everything counts as speech', () {
    final audio = recording([(5, true)]);
    final levels = measure(audio);
    expect(levels.threshold, isNull);
    expect(levels.speech(0, audio.length), (0, audio.length));
  });
}
