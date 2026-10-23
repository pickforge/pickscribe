import 'package:flutter/material.dart';

import '../models.dart';
import '../services.dart';
import 'models_screen.dart';
import 'theme.dart';
import 'widgets.dart';

/// The three things dictation needs: the microphone, the bubble service, and a model. It is
/// the first screen until the user finishes or skips it, and History links back to it.
class OnboardingScreen extends StatelessWidget {
  const OnboardingScreen({super.key, this.first = false});

  /// True when it is the app's first screen rather than a page over History.
  final bool first;

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    return Scaffold(
      appBar: first ? null : AppBar(),
      body: SafeArea(
        child: ListenableBuilder(
          listenable: Listenable.merge([services.system, services.models, services.settings]),
          builder: (context, _) {
            final status = services.system.status;
            final current = modelByFile(services.settings.settings.model);
            final hasModel = current != null && services.models.isDownloaded(current);
            final progress = services.models.progressOf(recommendedModel);
            final error = services.models.errorOf(recommendedModel);
            final ready = status.microphone && status.service && hasModel;
            return ListView(
              padding: const EdgeInsets.fromLTRB(20, 24, 20, 24),
              children: [
                const Align(alignment: Alignment.centerLeft, child: BrandMark(size: 64)),
                const SizedBox(height: 16),
                Text('Set up PickScribe', style: Theme.of(context).textTheme.headlineMedium),
                const SizedBox(height: 28),
                _Step(
                  number: 1,
                  title: 'Allow the microphone',
                  done: status.microphone,
                  action: 'Allow',
                  onPressed: services.system.bridge.requestMicrophone,
                ),
                _Step(
                  number: 2,
                  title: 'Turn on the bubble',
                  detail: 'In Accessibility, open Installed apps, then PickScribe dictation.',
                  done: status.service,
                  action: 'Open',
                  onPressed: services.system.bridge.openAccessibilitySettings,
                ),
                _Step(
                  number: 3,
                  title: 'Download a model',
                  detail: hasModel
                      ? current.name
                      : '${recommendedModel.name}, ${recommendedModel.size}. Use Wi-Fi.',
                  done: hasModel,
                  action: 'Download',
                  busy: progress != null,
                  onPressed: () => downloadAndUse(services, recommendedModel),
                  footer: progress != null
                      ? LinearProgressIndicator(value: progress)
                      : error != null
                      ? Text(error, style: const TextStyle(color: Palette.danger, fontSize: 13))
                      : null,
                ),
                const SizedBox(height: 20),
                FilledButton(
                  onPressed: ready ? () => _finish(context, services) : null,
                  child: const Text('Done'),
                ),
                if (first && !ready) ...[
                  const SizedBox(height: 8),
                  TextButton(
                    onPressed: () => _finish(context, services),
                    child: const Text('Skip for now', style: TextStyle(color: Palette.muted)),
                  ),
                ],
              ],
            );
          },
        ),
      ),
    );
  }

  void _finish(BuildContext context, Services services) {
    if (first) {
      services.finishOnboarding();
    } else {
      Navigator.of(context).pop();
    }
  }
}

class _Step extends StatelessWidget {
  const _Step({
    required this.number,
    required this.title,
    required this.done,
    required this.action,
    required this.onPressed,
    this.detail,
    this.busy = false,
    this.footer,
  });

  final int number;
  final String title;
  final String? detail;
  final bool done;
  final String action;
  final VoidCallback onPressed;
  final bool busy;
  final Widget? footer;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: SurfaceCard(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 16, 16, 16),
          child: Column(
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  AnimatedContainer(
                    duration: const Duration(milliseconds: 200),
                    width: 32,
                    height: 32,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      color: done ? Palette.ember : Palette.raised,
                    ),
                    child: done
                        ? const Icon(Icons.check_rounded, size: 20, color: Palette.canvas)
                        : Text('$number', style: const TextStyle(fontFamily: mono, fontSize: 14)),
                  ),
                  const SizedBox(width: 14),
                  Expanded(
                    child: _StepTitle(title: title, detail: detail, done: done),
                  ),
                ],
              ),
              if (footer != null && !done)
                Padding(padding: const EdgeInsets.fromLTRB(46, 14, 4, 0), child: footer),
              if (!done)
                Padding(
                  padding: const EdgeInsets.only(left: 46, top: 14),
                  child: _StepButton(label: action, onPressed: busy ? null : onPressed),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _StepTitle extends StatelessWidget {
  const _StepTitle({required this.title, required this.detail, required this.done});

  final String title;
  final String? detail;
  final bool done;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          title,
          style: TextStyle(
            fontSize: 16,
            fontWeight: FontWeight.w600,
            color: done ? Palette.muted : Palette.text,
          ),
        ),
        if (detail != null) ...[
          const SizedBox(height: 3),
          Text(detail!, style: const TextStyle(fontSize: 13, color: Palette.muted, height: 1.35)),
        ],
      ],
    );
  }
}

class _StepButton extends StatelessWidget {
  const _StepButton({required this.label, required this.onPressed});

  final String label;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment.centerLeft,
      child: FilledButton(
        style: FilledButton.styleFrom(minimumSize: const Size(0, 42)),
        onPressed: onPressed,
        child: Text(label),
      ),
    );
  }
}
