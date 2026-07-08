import { desktopApiAvailable } from "./api";

export type HostPlatform = "macos" | "windows" | "linux" | "web";

let cached: HostPlatform | undefined;

export function hostPlatform(): HostPlatform {
  if (cached) {
    return cached;
  }
  if (!desktopApiAvailable()) {
    return (cached = "web");
  }
  const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";
  if (/Macintosh|Mac OS X/.test(ua)) {
    cached = "macos";
  } else if (/Windows/.test(ua)) {
    cached = "windows";
  } else {
    cached = "linux";
  }
  return cached;
}
