# Runbook: archive PickScribe

For Elberte. Nothing here has been run by an agent. Run it after the retirement
PR is merged.

## Checklist before archiving

- [ ] The retirement PR is merged into `main`.
- [ ] The README banner is visible on the default branch:
      https://github.com/pickforge/pickscribe
- [ ] The last release still lists its assets (AppImage, deb, app.tar.gz, sigs,
      latest.json): https://github.com/pickforge/pickscribe/releases/tag/v0.2.1

## Archive the repository

```sh
gh repo archive pickforge/pickscribe --yes
```

The repository becomes read-only. Code, issues and releases stay visible and the
release assets stay downloadable. `gh repo unarchive pickforge/pickscribe` undoes
it if needed.

PickScribe has no published npm package, so there is nothing to deprecate.
