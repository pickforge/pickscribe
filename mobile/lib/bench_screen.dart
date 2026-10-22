import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

import 'models.dart';
import 'pipeline.dart';
import 'wav.dart';
import 'whisper.dart';

const _recorder = MethodChannel('pickscribe/recorder');
const _system = MethodChannel('pickscribe/system');
const _sampleRate = 16000;

/// Measures each downloaded model on the JFK sample and on a recording of your own.
class BenchScreen extends StatefulWidget {
  const BenchScreen({super.key});

  @override
  State<BenchScreen> createState() => _BenchScreenState();
}

class _BenchScreenState extends State<BenchScreen> with WidgetsBindingObserver {
  Directory? _dir;
  final _progress = <String, double>{};
  final _log = <String>[];
  Float32List? _recording;
  bool _listening = false;
  bool _busy = false;
  int _threads = 6;
  int _beam = 5;
  Map<Object?, Object?> _status = const {};
  DictationConfig? _config;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _refreshStatus();
    DictationConfig.load().then((config) => setState(() {
      _config = config;
      _beam = config?.beamSize ?? _beam;
      _threads = config?.threads ?? _threads;
    }));
    getApplicationSupportDirectory().then((dir) {
      Directory('${dir.path}/models').createSync(recursive: true);
      setState(() => _dir = Directory('${dir.path}/models'));
    });
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  // Permissions change in the system settings, so check again on return.
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) _refreshStatus();
  }

  Future<void> _refreshStatus() async {
    final status = await _system.invokeMapMethod<Object?, Object?>('status');
    setState(() => _status = status ?? const {});
  }

  Future<void> _choose(WhisperModel model) async {
    final config = DictationConfig(model: model.file, beamSize: _beam, threads: _threads);
    await config.save();
    setState(() => _config = config);
  }

  bool _downloaded(WhisperModel model) =>
      _dir != null && File('${_dir!.path}/${model.file}').existsSync();

  Future<void> _download(WhisperModel model) async {
    setState(() => _progress[model.file] = 0);
    try {
      await download(
        model,
        _dir!,
        onProgress: (received) =>
            setState(() => _progress[model.file] = received / model.bytes),
      );
    } catch (error) {
      _write('${model.file}: $error');
    } finally {
      setState(() => _progress.remove(model.file));
    }
  }

  Future<void> _toggleRecording() async {
    try {
      if (_listening) {
        final samples = await _recorder.invokeMethod<Float32List>('stop');
        setState(() {
          _listening = false;
          _recording = samples;
        });
        _write('Recorded ${(samples!.length / _sampleRate).toStringAsFixed(1)} s');
      } else {
        await _recorder.invokeMethod<void>('start');
        setState(() => _listening = true);
      }
    } on PlatformException catch (error) {
      _write(error.message ?? error.code);
    }
  }

  Future<void> _runAll(Float32List audio, String name) async {
    setState(() => _busy = true);
    final seconds = audio.length / _sampleRate;
    _write('$name, ${seconds.toStringAsFixed(1)} s, $_threads threads');
    for (final model in models.where(_downloaded)) {
      final clock = Stopwatch()..start();
      final Whisper whisper;
      try {
        whisper = await Whisper.load('${_dir!.path}/${model.file}');
      } catch (error) {
        _write('${model.file}: $error');
        continue;
      }
      final loadMs = clock.elapsedMilliseconds;
      for (final beam in [1, 5]) {
        clock.reset();
        try {
          final result = await whisper.transcribe(
            audio,
            languages: const ['pt', 'en'],
            threads: _threads,
            beamSize: beam,
          );
          final ms = clock.elapsedMilliseconds;
          _write(
            '${model.file} beam $beam: load $loadMs ms, '
            '$ms ms (${(ms / 1000 / seconds).toStringAsFixed(2)}x), '
            '${result.language}: ${result.text}',
          );
        } catch (error) {
          _write('${model.file} beam $beam: $error');
        }
      }
      whisper.dispose();
    }
    setState(() => _busy = false);
  }

  Future<void> _runSample() async {
    final bytes = await rootBundle.load('assets/samples/jfk.wav');
    await _runAll(decodePcm16Wav(bytes.buffer.asUint8List()), 'JFK sample');
  }

  void _write(String line) => setState(() => _log.insert(0, line));

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('PickScribe benchmark')),
      body: _dir == null
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.all(16),
              children: [
                _permissionRow(
                  'Microphone',
                  _status['microphone'] == true,
                  () => _system.invokeMethod<void>('requestMicrophone'),
                ),
                _permissionRow(
                  'Bubble service',
                  _status['service'] == true,
                  () => _system.invokeMethod<void>('openAccessibilitySettings'),
                ),
                const Divider(),
                for (final model in models) _modelTile(model),
                const SizedBox(height: 8),
                Row(
                  children: [
                    const Text('Threads'),
                    const SizedBox(width: 12),
                    DropdownButton<int>(
                      value: _threads,
                      items: [
                        for (final n in [2, 4, 6, 8])
                          DropdownMenuItem(value: n, child: Text('$n')),
                      ],
                      onChanged: _busy ? null : (n) => _setDecoding(threads: n),
                    ),
                    const SizedBox(width: 24),
                    const Text('Beam'),
                    const SizedBox(width: 12),
                    DropdownButton<int>(
                      value: _beam,
                      items: [
                        for (final n in [1, 5]) DropdownMenuItem(value: n, child: Text('$n')),
                      ],
                      onChanged: _busy ? null : (n) => _setDecoding(beam: n),
                    ),
                  ],
                ),
                Wrap(
                  spacing: 8,
                  children: [
                    FilledButton(
                      onPressed: _busy ? null : _toggleRecording,
                      child: Text(_listening ? 'Stop' : 'Record'),
                    ),
                    OutlinedButton(
                      onPressed: _busy || _listening ? null : _runSample,
                      child: const Text('Run JFK sample'),
                    ),
                    OutlinedButton(
                      onPressed: _busy || _listening || _recording == null
                          ? null
                          : () => _runAll(_recording!, 'Recording'),
                      child: const Text('Run recording'),
                    ),
                  ],
                ),
                if (_busy) const LinearProgressIndicator(),
                const Divider(),
                for (final line in _log) SelectableText(line),
              ],
            ),
    );
  }

  void _setDecoding({int? threads, int? beam}) {
    setState(() {
      _threads = threads ?? _threads;
      _beam = beam ?? _beam;
    });
    final model = _config?.model;
    if (model != null) {
      _choose(models.firstWhere((m) => m.file == model));
    }
  }

  Widget _permissionRow(String label, bool granted, VoidCallback onFix) {
    return ListTile(
      contentPadding: EdgeInsets.zero,
      title: Text(label),
      trailing: granted
          ? const Icon(Icons.check)
          : TextButton(onPressed: onFix, child: const Text('Allow')),
    );
  }

  Widget _modelTile(WhisperModel model) {
    final progress = _progress[model.file];
    final downloaded = _downloaded(model);
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: Icon(
        _config?.model == model.file ? Icons.radio_button_checked : Icons.radio_button_off,
        color: downloaded ? null : Theme.of(context).disabledColor,
      ),
      onTap: downloaded ? () => _choose(model) : null,
      title: Text(model.label),
      subtitle: progress == null ? null : LinearProgressIndicator(value: progress),
      trailing: downloaded
          ? const Icon(Icons.check)
          : IconButton(
              icon: const Icon(Icons.download),
              onPressed: progress == null ? () => _download(model) : null,
            ),
    );
  }
}
