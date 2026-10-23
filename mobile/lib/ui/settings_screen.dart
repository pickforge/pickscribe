import 'package:flutter/material.dart';

import '../bridge.dart';
import '../models.dart';
import '../services.dart';
import '../settings.dart';
import 'blocked_apps_screen.dart';
import 'format.dart';
import 'models_screen.dart';
import 'speed_test_screen.dart';
import 'theme.dart';
import 'widgets.dart';

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    return ListenableBuilder(
      listenable: Listenable.merge([services.system, services.settings, services.models]),
      builder: (context, _) {
        final system = services.system;
        final settings = services.settings.settings;
        final model = modelByFile(settings.model);
        final snoozed = system.bubble.snoozedUntil;
        final now = DateTime.now();
        return ListView(
          padding: const EdgeInsets.only(bottom: 24),
          children: [
            const PageHeader('Settings'),
            const SizedBox(height: 8),
            Section(
              title: 'Bubble',
              children: [
                const _BubbleLookEditor(),
                if (snoozed != null && snoozed.isAfter(now))
                  SettingRow(
                    icon: Icons.bedtime_outlined,
                    title: 'Paused ${until(snoozed, now)}',
                    trailing: TextButton(onPressed: system.resume, child: const Text('Resume')),
                  ),
                SettingRow(
                  icon: Icons.block_rounded,
                  title: 'Hidden in apps',
                  value: system.bubble.blocked.isEmpty ? 'None' : '${system.bubble.blocked.length}',
                  onTap: () => _push(context, const BlockedAppsScreen()),
                ),
              ],
            ),
            Section(
              title: 'Transcription',
              children: [
                SettingRow(
                  icon: Icons.memory_rounded,
                  title: 'Model',
                  value: model != null && services.models.isDownloaded(model) ? model.name : 'None',
                  onTap: () => _push(context, const ModelsScreen()),
                ),
                SettingRow(
                  icon: Icons.translate_rounded,
                  title: 'Languages',
                  value: settings.languages.map((code) => languageNames[code]).join(', '),
                  onTap: () => _pickLanguages(context, services.settings),
                ),
                SettingRow(
                  icon: Icons.speed_rounded,
                  title: 'Speed test',
                  onTap: () => _push(context, const SpeedTestScreen()),
                ),
              ],
            ),
            _Permissions(system: system),
            Section(
              title: 'About',
              children: [
                SettingRow(title: 'Version', value: system.status.version),
                SettingRow(
                  title: 'Open-source licenses',
                  onTap: () => showLicensePage(
                    context: context,
                    applicationName: 'PickScribe',
                    applicationVersion: system.status.version,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            const Center(child: BrandMark(size: 40, faded: true)),
            const SizedBox(height: 8),
            const Text(
              'Audio and text stay on this phone.',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 12.5, color: Palette.faint),
            ),
          ],
        );
      },
    );
  }

  static void _push(BuildContext context, Widget screen) =>
      Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => screen));

  static Future<void> _pickLanguages(BuildContext context, SettingsStore store) {
    return showModalBottomSheet<void>(
      context: context,
      builder: (_) => _LanguagesSheet(store: store),
    );
  }
}

class _Permissions extends StatelessWidget {
  const _Permissions({required this.system});

  final SystemState system;

  @override
  Widget build(BuildContext context) {
    return Section(
      title: 'Permissions',
      children: [
        SettingRow(
          icon: Icons.mic_none_rounded,
          title: 'Microphone',
          trailing: system.status.microphone
              ? const _Granted('Allowed')
              : TextButton(onPressed: system.bridge.requestMicrophone, child: const Text('Allow')),
        ),
        SettingRow(
          icon: Icons.bubble_chart_outlined,
          title: 'Bubble service',
          trailing: system.status.service
              ? const _Granted('On')
              : TextButton(
                  onPressed: system.bridge.openAccessibilitySettings,
                  child: const Text('Turn on'),
                ),
        ),
      ],
    );
  }
}

class _Granted extends StatelessWidget {
  const _Granted(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(label, style: const TextStyle(color: Palette.muted, fontSize: 14)),
        const SizedBox(width: 6),
        const Icon(Icons.check_circle_rounded, color: Palette.ember, size: 20),
        const SizedBox(width: 8),
      ],
    );
  }
}

/// Size and opacity sliders over a preview of the bubble on a text field.
class _BubbleLookEditor extends StatefulWidget {
  const _BubbleLookEditor();

  @override
  State<_BubbleLookEditor> createState() => _BubbleLookEditorState();
}

class _BubbleLookEditorState extends State<_BubbleLookEditor> {
  double? _size;
  double? _opacity;

  @override
  Widget build(BuildContext context) {
    final system = ServicesScope.of(context).system;
    final size = _size ?? system.bubble.size.toDouble();
    final opacity = _opacity ?? system.bubble.opacity;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
      child: Column(
        children: [
          _Preview(size: size, opacity: opacity),
          const SizedBox(height: 12),
          _SliderRow(
            label: 'Size',
            value: size,
            min: BubbleLook.minSize.toDouble(),
            max: BubbleLook.maxSize.toDouble(),
            divisions: (BubbleLook.maxSize - BubbleLook.minSize) ~/ 4,
            display: '${size.round()} dp',
            onChanged: (value) => setState(() => _size = value),
            onChangeEnd: (value) async {
              await system.setLook(size: value.round());
              if (mounted) setState(() => _size = null);
            },
          ),
          _SliderRow(
            label: 'Opacity',
            value: opacity,
            min: BubbleLook.minOpacity,
            max: 1,
            divisions: 14,
            display: '${(opacity * 100).round()}%',
            onChanged: (value) => setState(() => _opacity = value),
            onChangeEnd: (value) async {
              await system.setLook(opacity: value);
              if (mounted) setState(() => _opacity = null);
            },
          ),
        ],
      ),
    );
  }
}

class _Preview extends StatelessWidget {
  const _Preview({required this.size, required this.opacity});

  final double size;
  final double opacity;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 128,
      decoration: BoxDecoration(
        color: Palette.canvas,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: Palette.border),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 14),
      child: Row(
        children: [
          Expanded(
            child: Container(
              height: 44,
              padding: const EdgeInsets.symmetric(horizontal: 14),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: Palette.raised,
                borderRadius: BorderRadius.circular(22),
              ),
              child: Row(
                children: [
                  Container(width: 2, height: 18, color: Palette.ember),
                  const SizedBox(width: 8),
                  const Text('Message', style: TextStyle(color: Palette.faint, fontSize: 15)),
                ],
              ),
            ),
          ),
          SizedBox(
            width: 92,
            child: Center(
              child: BubbleGlyph(size: size, opacity: opacity),
            ),
          ),
        ],
      ),
    );
  }
}

class _SliderRow extends StatelessWidget {
  const _SliderRow({
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.divisions,
    required this.display,
    required this.onChanged,
    required this.onChangeEnd,
  });

  final String label;
  final double value;
  final double min;
  final double max;
  final int divisions;
  final String display;
  final ValueChanged<double> onChanged;
  final ValueChanged<double> onChangeEnd;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Column(
        children: [
          Row(
            children: [
              Text(label, style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w500)),
              const Spacer(),
              Text(
                display,
                style: const TextStyle(fontFamily: mono, fontSize: 13, color: Palette.muted),
              ),
            ],
          ),
          Slider(
            value: value.clamp(min, max),
            min: min,
            max: max,
            divisions: divisions,
            semanticFormatterCallback: (_) => display,
            onChanged: onChanged,
            onChangeEnd: onChangeEnd,
          ),
        ],
      ),
    );
  }
}

class _LanguagesSheet extends StatelessWidget {
  const _LanguagesSheet({required this.store});

  final SettingsStore store;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final chosen = store.settings.languages;
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(20, 0, 20, 20),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Languages', style: Theme.of(context).textTheme.titleLarge),
                const SizedBox(height: 6),
                const Text(
                  'Each dictation uses the most likely of these.',
                  style: TextStyle(color: Palette.muted),
                ),
                const SizedBox(height: 16),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final entry in languageNames.entries)
                      FilterChip(
                        label: Text(entry.value),
                        selected: chosen.contains(entry.key),
                        onSelected: (selected) {
                          final next = [
                            for (final code in languageNames.keys)
                              if (code == entry.key ? selected : chosen.contains(code)) code,
                          ];
                          // At least one language stays on.
                          if (next.isEmpty) return;
                          store.update(store.settings.copyWith(languages: next));
                        },
                      ),
                  ],
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}
