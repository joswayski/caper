export type DownloadPlatform = "macos" | "windows" | "linux-deb" | "linux";

const release = "https://github.com/joswayski/caper/releases/download/native-latest";

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
    url: "https://github.com/joswayski/caper/releases/tag/native-latest",
  },
};
export const intelMacDownload = `${release}/Caper-macOS-Intel.dmg`;

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
  if (mobile || /iphone|ipad|ipod|android|cros|chrome os/i.test(hints) || (/mac/i.test(hints) && maxTouchPoints > 1))
    return null;
  if (/mac/i.test(hints)) return "macos";
  if (/win/i.test(hints)) return "windows";
  if (/linux/i.test(hints)) {
    return /ubuntu|debian|linux mint|pop!_os|elementary/i.test(userAgent) ? "linux-deb" : "linux";
  }
  return null;
}
