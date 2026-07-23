import { createFlags } from "@pickforge/flags";

const definitions = {
  studioUpdateDialog: {
    description: "Shared Pickforge Studio update dialog (pickforge-platform#36)",
    default: false,
  },
} as const;

const flags = createFlags(definitions);

export function studioUpdateDialogEnabled(): boolean {
  return flags.isEnabled("studioUpdateDialog");
}
