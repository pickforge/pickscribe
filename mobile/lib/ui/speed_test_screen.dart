import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../models.dart';
import '../pipeline.dart';
import '../services.dart';
import '../wav.dart';
import '../whisper.dart';
import 'theme.dart';
import 'widgets.dart';

const _recorder = MethodChannel('pickscribe/recorder');

/// Times each downloaded model on a sample or on a recording, with the current settings or
/// other thread and beam values. Nothing here is saved.
class SpeedTestScreen extends StatefulWidget {
  const SpeedTestScreen({super.key});

  @override
  State<SpeedTestScreen> createState() => _SpeedTestScreenState();
}

class _Result {
  const _Result(this.title, this.detail, this.text, {this.error = false});

  final String title;
  final String detail;
  final String text;
  final bool error;
}

class _SpeedTestScreenState extends State<SpeedTestScreen> {
  final _results = <_Result>[];
  Float32List? _recording;
  bool _listening = false;
  bool _busy = false;
  late int _threads = ServicesScope.of(context).settings.settings.threads;
  late int _beam = ServicesScope.of(context).settings.settings.beamSize;

  @override
  void dispose() {
    if (_listening) _recorder.invokeMethod<Float32List>('stop');
    super.dispose();
  }

  Future<void> _toggleRecording() async {
    try {
      if (_listening) {
        final samples = await _recorder.invokeMethod<Float32List>('stop');
        setState(() {
          _listening = false;
          _recording = samples;
        });
      } else {
        await _recorder.invokeMethod<void>('start');
        setState(() => _listening = true);
      }
    } on PlatformException catch (error) {
      _add(_Result('Recording', error.message ?? error.code, '', error: true));
    }
  }

  Future<void> _runSample() async {
    final bytes = await rootBundle.load('assets/samples/jfk.wav');
    await _run(decodePcm16Wav(bytes.buffer.asUint8List()), 'Sample');
  }

  Future<void> _run(Float32List audio, String name) async {
    final services = ServicesScope.of(context);
    final languages = services.settings.settings.languages;
    setState(() => _busy = true);
    final seconds = audio.length / sampleRate;
    for (final model in models.where(services.models.isDownloaded)) {
      final clock = Stopwatch()..start();
      try {
        final whisper = await Whisper.load(services.models.fileOf(model).path);
        final loadMs = clock.elapsedMilliseconds;
        clock.reset();
        try {
          final result = await whisper.transcribe(
            audio,
            languages: languages,
            threads: _threads,
            beamSize: _beam,
          );
          final ms = clock.elapsedMilliseconds;
          _add(
            _Result(
              '${model.name} · ${(ms / 1000).toStringAsFixed(1)} s',
              '$name ${seconds.toStringAsFixed(1)} s, ${(ms / 1000 / seconds).toStringAsFixed(2)}× real time, '
                  'load ${loadMs}ms, $_threads threads, beam $_beam, ${result.language}',
              result.text,
            ),
          );
        } finally {
          whisper.dispose();
        }
      } on WhisperException catch (error) {
        _add(_Result(model.name, error.message, '', error: true));
      }
    }
    if (mounted) setState(() => _busy = false);
  }

  void _add(_Result result) {
    if (mounted) setState(() => _results.insert(0, result));
  }

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    final ready = models.any(services.models.isDownloaded);
    return Scaffold(
      appBar: AppBar(title: const Text('Speed test')),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
        children: [
          Section(
            padding: const EdgeInsets.only(bottom: 16),
            children: [
              _Choice(
                label: 'Threads',
                values: const [2, 4, 6, 8],
                value: _threads,
                onChanged: _busy ? null : (value) => setState(() => _threads = value),
              ),
              _Choice(
                label: 'Beam',
                values: const [1, 5],
                value: _beam,
                onChanged: _busy ? null : (value) => setState(() => _beam = value),
              ),
            ],
          ),
          Row(
            children: [
              Expanded(
                child: FilledButton.icon(
                  icon: Icon(_listening ? Icons.stop_rounded : Icons.mic_rounded),
                  label: Text(_listening ? 'Stop' : 'Record'),
                  onPressed: _busy ? null : _toggleRecording,
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: OutlinedButton(
                  onPressed: !ready || _busy || _listening || _recording == null
                      ? null
                      : () => _run(_recording!, 'Recording'),
                  child: const Text('Test recording'),
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          OutlinedButton(
            onPressed: !ready || _busy || _listening ? null : _runSample,
            child: const Text('Test the English sample'),
          ),
          if (_busy)
            const Padding(padding: EdgeInsets.only(top: 16), child: LinearProgressIndicator()),
          const SizedBox(height: 16),
          for (final result in _results) _ResultCard(result: result),
        ],
      ),
    );
  }
}

class _Choice extends StatelessWidget {
  const _Choice({
    required this.label,
    required this.values,
    required this.value,
    required this.onChanged,
  });

  final String label;
  final List<int> values;
  final int value;
  final ValueChanged<int>? onChanged;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
      child: Row(
        children: [
          Text(label, style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w500)),
          const Spacer(),
          SegmentedButton<int>(
            showSelectedIcon: false,
            style: SegmentedButton.styleFrom(
              selectedBackgroundColor: Palette.emberWash,
              selectedForegroundColor: Palette.ember,
              side: const BorderSide(color: Palette.border),
              visualDensity: VisualDensity.compact,
            ),
            segments: [for (final v in values) ButtonSegment(value: v, label: Text('$v'))],
            selected: {value},
            onSelectionChanged: onChanged == null ? null : (selected) => onChanged!(selected.first),
          ),
        ],
      ),
    );
  }
}

class _ResultCard extends StatelessWidget {
  const _ResultCard({required this.result});

  final _Result result;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: SurfaceCard(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                result.title,
                style: TextStyle(
                  fontSize: 15,
                  fontWeight: FontWeight.w600,
                  color: result.error ? Palette.danger : Palette.text,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                result.detail,
                style: const TextStyle(
                  fontFamily: mono,
                  fontSize: 12,
                  color: Palette.muted,
                  height: 1.4,
                ),
              ),
              if (result.text.isNotEmpty) ...[
                const SizedBox(height: 10),
                Text(result.text, style: const TextStyle(fontSize: 14, height: 1.45)),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
