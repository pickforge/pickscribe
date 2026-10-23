import 'dart:math';
import 'dart:typed_data';

/// Tracks how loud a recording is, 30 ms at a time, to find the pauses where a long dictation
/// can be cut into parts that are transcribed while the user keeps speaking.
class SpeechLevels {
  static const frame = 480;

  /// A part is at least this long before a pause may end it, since Whisper encodes at least
  /// 7 seconds anyway and short parts lose context.
  static const minPart = 6 * 16000;

  /// A part with no pause is cut at its quietest moment after this long.
  static const maxPart = 20 * 16000;

  // A pause that ends a part, and the quiet kept around trimmed speech.
  static const _pauseFrames = 20;
  static const _padding = 6400;

  // A recording with nothing louder than this is silence, even with no contrast in it.
  static const _silence = -50.0;

  final _levels = <double>[];

  /// Measures the complete frames of [audio] that are not measured yet.
  void add(Float32List audio, int length) {
    for (var start = _levels.length * frame; start + frame <= length; start += frame) {
      var sum = 0.0;
      for (var i = start; i < start + frame; i++) {
        sum += audio[i] * audio[i];
      }
      _levels.add(10 * log(sum / frame + 1e-10) / ln10);
    }
  }

  /// The loudness between silence and speech in this recording, in dB, or null when the two
  /// are too close to tell apart, as in loud noise or a recording with no pause. A quiet
  /// recording with no contrast is all silence so far.
  double? get threshold {
    if (_levels.length < 33) return null;
    final sorted = [..._levels]..sort();
    final quiet = sorted[sorted.length ~/ 10];
    final loud = sorted[sorted.length * 9 ~/ 10];
    if (loud - quiet < 15) return loud < _silence ? _silence : null;
    return min(quiet + 12, loud - 10);
  }

  /// Where to end the part that starts at sample [start], or null to keep listening. A part
  /// ends in the middle of the first long pause after [minPart], or at its quietest moment
  /// once it reaches [maxPart].
  int? cut(int start) {
    final first = start ~/ frame;
    if ((_levels.length - first) * frame < minPart) return null;
    final threshold = this.threshold;
    if (threshold != null) {
      var quiet = 0;
      for (var i = first + minPart ~/ frame; i < _levels.length; i++) {
        quiet = _levels[i] < threshold ? quiet + 1 : 0;
        if (quiet == _pauseFrames) return (i + 1 - _pauseFrames ~/ 2) * frame;
      }
    }
    if ((_levels.length - first) * frame < maxPart) return null;
    return _quietest(first + minPart ~/ frame, _levels.length) * frame;
  }

  /// The speech in samples [start] to [end], with a little quiet kept around it, or null when
  /// that span is silent. Without a clear threshold, the whole span counts as speech.
  (int, int)? speech(int start, int end) {
    final threshold = this.threshold;
    if (threshold == null) return (start, end);
    final last = min(end ~/ frame, _levels.length);
    int? from;
    int? to;
    for (var i = start ~/ frame; i < last; i++) {
      if (_levels[i] < threshold) continue;
      from ??= i;
      to = i + 1;
    }
    if (from == null || to == null) return null;
    final speechEnd = to == last ? end : to * frame + _padding;
    return (max(start, from * frame - _padding), min(end, speechEnd));
  }

  // The start of the quietest 300 ms between frames [from] and [to].
  int _quietest(int from, int to) {
    const window = 10;
    var best = from;
    var bestSum = double.infinity;
    var sum = 0.0;
    for (var i = from; i < to; i++) {
      sum += _levels[i];
      if (i - from >= window) sum -= _levels[i - window];
      if (i - from >= window - 1 && sum < bestSum) {
        bestSum = sum;
        best = i + 1 - window;
      }
    }
    return best + window ~/ 2;
  }
}
