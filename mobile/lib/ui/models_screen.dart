import 'package:flutter/material.dart';

import '../models.dart';
import '../services.dart';
import 'theme.dart';
import 'widgets.dart';

class ModelsScreen extends StatelessWidget {
  const ModelsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Model')),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
        children: [for (final model in models) _ModelCard(model: model)],
      ),
    );
  }
}

/// Downloads [model] and makes it the dictation model when none works yet.
Future<void> downloadAndUse(Services services, WhisperModel model) async {
  if (!await services.models.download(model)) return;
  final current = modelByFile(services.settings.settings.model);
  if (current == null || !services.models.isDownloaded(current)) {
    await services.settings.update(services.settings.settings.copyWith(model: model.file));
  }
}

class _ModelCard extends StatelessWidget {
  const _ModelCard({required this.model});

  final WhisperModel model;

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    return ListenableBuilder(
      listenable: Listenable.merge([services.models, services.settings]),
      builder: (context, _) {
        final manager = services.models;
        final downloaded = manager.isDownloaded(model);
        final progress = manager.progressOf(model);
        final error = manager.errorOf(model);
        final selected = downloaded && services.settings.settings.model == model.file;
        return Padding(
          padding: const EdgeInsets.only(bottom: 10),
          child: Material(
            color: selected ? Palette.emberWash : Palette.surface,
            clipBehavior: Clip.antiAlias,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(20),
              side: BorderSide(color: selected ? Palette.ember : Palette.border),
            ),
            child: InkWell(
              onTap: downloaded && !selected
                  ? () => services.settings.update(
                      services.settings.settings.copyWith(model: model.file),
                    )
                  : null,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 14, 8, 14),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Icon(
                          selected
                              ? Icons.radio_button_checked_rounded
                              : Icons.radio_button_off_rounded,
                          color: selected
                              ? Palette.ember
                              : (downloaded ? Palette.muted : Palette.raised),
                          size: 22,
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                model.name,
                                style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
                              ),
                              const SizedBox(height: 4),
                              Wrap(
                                spacing: 8,
                                runSpacing: 4,
                                crossAxisAlignment: WrapCrossAlignment.center,
                                children: [
                                  Text(
                                    model.size,
                                    style: const TextStyle(
                                      fontFamily: mono,
                                      fontSize: 12.5,
                                      color: Palette.muted,
                                    ),
                                  ),
                                  if (model == recommendedModel) const Tag('Recommended'),
                                ],
                              ),
                            ],
                          ),
                        ),
                        _Actions(
                          model: model,
                          downloaded: downloaded,
                          downloading: progress != null,
                        ),
                      ],
                    ),
                    Padding(
                      padding: const EdgeInsets.only(left: 34, top: 10),
                      child: Wrap(
                        spacing: 16,
                        runSpacing: 6,
                        children: [
                          Meter(label: 'Speed', value: model.speed),
                          Meter(label: 'Accuracy', value: model.accuracy),
                        ],
                      ),
                    ),
                    if (progress != null)
                      Padding(
                        padding: const EdgeInsets.fromLTRB(34, 14, 8, 2),
                        child: Row(
                          children: [
                            Expanded(child: LinearProgressIndicator(value: progress)),
                            const SizedBox(width: 12),
                            Text(
                              '${(progress * 100).floor()}%',
                              style: const TextStyle(
                                fontFamily: mono,
                                fontSize: 12,
                                color: Palette.muted,
                              ),
                            ),
                          ],
                        ),
                      ),
                    if (error != null)
                      Padding(
                        padding: const EdgeInsets.fromLTRB(34, 10, 8, 0),
                        child: Text(
                          error,
                          style: const TextStyle(color: Palette.danger, fontSize: 13),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class _Actions extends StatelessWidget {
  const _Actions({required this.model, required this.downloaded, required this.downloading});

  final WhisperModel model;
  final bool downloaded;
  final bool downloading;

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    if (downloading) {
      return IconButton(
        tooltip: 'Stop download',
        icon: const Icon(Icons.close_rounded),
        onPressed: () => services.models.cancel(model),
      );
    }
    if (!downloaded) {
      return IconButton(
        tooltip: 'Download',
        icon: const Icon(Icons.download_rounded, color: Palette.ember),
        onPressed: () => downloadAndUse(services, model),
      );
    }
    return PopupMenuButton<void>(
      tooltip: 'More',
      icon: const Icon(Icons.more_vert_rounded, color: Palette.muted),
      color: Palette.raised,
      itemBuilder: (context) => [
        PopupMenuItem(onTap: () => _confirmDelete(context, services), child: const Text('Delete')),
      ],
    );
  }

  Future<void> _confirmDelete(BuildContext context, Services services) async {
    final delete = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete ${model.name}?'),
        content: Text('This frees ${model.size}. You can download it again later.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          TextButton(
            style: TextButton.styleFrom(foregroundColor: Palette.danger),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (delete == true) await services.models.delete(model);
  }
}
