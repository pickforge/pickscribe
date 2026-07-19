function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function equal(left: unknown, right: unknown): boolean {
  if (isRecord(left) && isRecord(right)) {
    const leftKeys = Object.keys(left);
    const rightKeys = Object.keys(right);
    return (
      leftKeys.length === rightKeys.length &&
      leftKeys.every((key) => key in right && equal(left[key], right[key]))
    );
  }
  return Object.is(left, right);
}

export function mergeExternalSettings<T>(baseline: T, local: T, external: T): T {
  if (isRecord(baseline) && isRecord(local) && isRecord(external)) {
    const merged: Record<string, unknown> = {};
    for (const key of new Set([
      ...Object.keys(baseline),
      ...Object.keys(local),
      ...Object.keys(external),
    ])) {
      merged[key] = mergeExternalSettings(baseline[key], local[key], external[key]);
    }
    return merged as T;
  }

  return (equal(local, baseline) ? external : local) as T;
}

export function shouldApplySaveResponse(
  eventRevisionAtStart: number,
  currentEventRevision: number
): boolean {
  return currentEventRevision === eventRevisionAtStart;
}

export interface ExternalSettingsResolution<T> {
  config: T;
  baseline: T;
  keptLocalChanges: boolean;
}

export function reconcileExternalSettings<T>(
  baseline: T,
  local: T,
  external: T
): ExternalSettingsResolution<T> {
  if (equal(local, baseline) || equal(local, external)) {
    return { config: external, baseline: external, keptLocalChanges: false };
  }
  return {
    config: mergeExternalSettings(baseline, local, external),
    baseline: external,
    keptLocalChanges: true,
  };
}
