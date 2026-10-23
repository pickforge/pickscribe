import 'package:flutter/material.dart';

import '../bridge.dart';
import '../services.dart';
import 'theme.dart';
import 'widgets.dart';

/// Apps where the bubble never shows.
class BlockedAppsScreen extends StatelessWidget {
  const BlockedAppsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final system = ServicesScope.of(context).system;
    return Scaffold(
      appBar: AppBar(title: const Text('Hidden in apps')),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: () =>
            Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => const _AppPicker())),
        icon: const Icon(Icons.add_rounded),
        label: const Text('Add apps'),
      ),
      body: ListenableBuilder(
        listenable: system,
        builder: (context, _) {
          final blocked = system.bubble.blocked;
          if (blocked.isEmpty) {
            return const EmptyState('The bubble shows in every app.');
          }
          return ListView(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 96),
            children: [
              SurfaceCard(
                child: Column(
                  children: [
                    for (var i = 0; i < blocked.length; i++) ...[
                      if (i > 0) const Divider(indent: 64, endIndent: 16),
                      _BlockedRow(blocked[i]),
                    ],
                  ],
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _BlockedRow extends StatelessWidget {
  const _BlockedRow(this.package);

  final String package;

  @override
  Widget build(BuildContext context) {
    final system = ServicesScope.of(context).system;
    return ListTile(
      minTileHeight: 60,
      leading: AppIcon(package, size: 32),
      title: Text(system.app(package)?.name ?? package),
      trailing: TextButton(onPressed: () => system.unblock(package), child: const Text('Show')),
    );
  }
}

class _AppPicker extends StatefulWidget {
  const _AppPicker();

  @override
  State<_AppPicker> createState() => _AppPickerState();
}

class _AppPickerState extends State<_AppPicker> {
  final _query = TextEditingController();
  late final Future<List<AppInfo>> _apps = ServicesScope.of(context).system.bridge.launchableApps();

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final system = ServicesScope.of(context).system;
    return Scaffold(
      appBar: AppBar(title: const Text('Add apps')),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
            child: TextField(
              controller: _query,
              decoration: const InputDecoration(
                hintText: 'Search apps',
                prefixIcon: Icon(Icons.search_rounded),
              ),
              onChanged: (_) => setState(() {}),
            ),
          ),
          Expanded(
            child: FutureBuilder(
              future: _apps,
              builder: (context, snapshot) {
                final apps = snapshot.data;
                if (apps == null) return const Center(child: CircularProgressIndicator());
                final query = _query.text.trim().toLowerCase();
                final shown = apps.where((app) => app.name.toLowerCase().contains(query)).toList();
                return ListenableBuilder(
                  listenable: system,
                  builder: (context, _) => ListView.builder(
                    padding: const EdgeInsets.only(bottom: 24),
                    itemCount: shown.length,
                    itemBuilder: (context, index) {
                      final app = shown[index];
                      final blocked = system.bubble.blocked.contains(app.package);
                      return CheckboxListTile(
                        value: blocked,
                        secondary: AppIcon(app.package, size: 32),
                        title: Text(app.name),
                        activeColor: Palette.ember,
                        checkColor: Palette.canvas,
                        onChanged: (block) =>
                            block == true ? system.block(app.package) : system.unblock(app.package),
                      );
                    },
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
