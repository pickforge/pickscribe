# PickScribe for Omarchy

This folder is an Omarchy shell plugin that shows PickScribe in the bar, with a panel for history, the dictionary, and timings.
`scripts/install` copies `manifest.json` and `Panel.qml` into `~/.config/omarchy/plugins/pickforge.pickscribe/` when Omarchy is present.
Enable it once with `omarchy plugin enable pickforge.pickscribe`. The icon appears in the right section of the bar.
To check the panel without the daemon or your screen, run `PATH=<dir with dev/pickscribe-stub named pickscribe>:$PATH scripts/omarchy-lab up`.
