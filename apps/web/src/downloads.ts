export type DesktopPlatform = "macos" | "windows" | "linux-deb" | "linux";
export type DownloadPlatform = DesktopPlatform | "ios" | "android";

const release = "https://github.com/joswayski/caper/releases/download/native-latest";
export const desktopRelease = "https://github.com/joswayski/caper/releases/tag/native-latest";

export const downloads = {
  macos: {
    label: "Download for macOS",
    detail: "Apple Silicon · .dmg",
    url: `${release}/Caper-macOS-Apple-Silicon.dmg`,
  },
  windows: { label: "Download for Windows", detail: "x64 · Setup.exe", url: `${release}/Caper-Windows-x64-Setup.exe` },
  "linux-deb": { label: "Download for Linux", detail: "x64 · .deb", url: `${release}/Caper-Linux-x64.deb` },
  linux: {
    label: "Linux downloads",
    detail: "Check compatibility",
    url: desktopRelease,
  },
};
export const intelMacDownload = `${release}/Caper-macOS-Intel.dmg`;

// Public phone betas. A link stays null until it accepts testers: the TestFlight
// public link once a build passes Beta App Review, and the Google Group once the
// Play closed test has an approved release. Android needs two steps because a
// closed test only admits group members, who then opt in on Google Play.
export const testFlightUrl: string | null = "https://testflight.apple.com/join/khGQMuYP";
export const androidTestersGroupUrl: string | null = null;
export const playTestingUrl = "https://play.google.com/apps/testing/chat.caper.android";

export const mobileBetas: Record<"ios" | "android", { label: string; url: string } | null> = {
  ios: testFlightUrl ? { label: "Get the iOS beta", url: testFlightUrl } : null,
  android: androidTestersGroupUrl ? { label: "Get the Android beta", url: androidTestersGroupUrl } : null,
};

// Unknown Linux distributions need the compatibility notes, not an archive dump.
export function detectDownloadPlatform({
  userAgent,
  platform = "",
  mobile,
  maxTouchPoints = 0,
}: {
  userAgent: string;
  platform?: string;
  mobile?: boolean;
  maxTouchPoints?: number;
}): DownloadPlatform | null {
  const hints = `${platform} ${userAgent}`;
  if (/iphone|ipad|ipod/i.test(hints) || (/mac/i.test(hints) && maxTouchPoints > 1)) return "ios";
  if (/android/i.test(hints)) return "android";
  if (mobile || /cros|chrome os/i.test(hints)) return null;
  if (/mac/i.test(hints)) return "macos";
  if (/win/i.test(hints)) return "windows";
  if (/linux/i.test(hints)) {
    return /ubuntu|debian|linux mint|pop!_os|elementary/i.test(userAgent) ? "linux-deb" : "linux";
  }
  return null;
}
