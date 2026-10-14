export type DesktopPlatform = "macos" | "windows" | "linux-deb" | "linux";
export type DownloadPlatform = DesktopPlatform | "ios" | "android";

const release = "https://github.com/joswayski/caper/releases/download/native-latest";
export const desktopRelease = "https://github.com/joswayski/caper/releases/tag/native-latest";

// Public phone betas. A link stays null until it accepts testers: the TestFlight
// public link once a build passes Beta App Review, and the Google Group once the
// Play closed test has an approved release. Android needs two steps because a
// closed test only admits group members, who then opt in on Google Play.
export const testFlightUrl: string | null = "https://testflight.apple.com/join/khGQMuYP";
export const androidTestersGroupUrl: string | null = "https://groups.google.com/g/caper-testers";
export const playTestingUrl = "https://play.google.com/apps/testing/chat.caper.android";

export const mobileBetas: Record<"ios" | "android", { label: string; url: string } | null> = {
  ios: testFlightUrl ? { label: "Get the iOS beta", url: testFlightUrl } : null,
  android: androidTestersGroupUrl ? { label: "Get the Android beta", url: androidTestersGroupUrl } : null,
};

type DownloadGroup = {
  platform: DownloadPlatform;
  label: string;
  links: { label: string; url: string }[];
  note?: string;
};

export const downloadGroups: readonly DownloadGroup[] = [
  {
    platform: "macos",
    label: "macOS",
    links: [
      { label: "Apple Silicon · .dmg", url: `${release}/Caper-macOS-Apple-Silicon.dmg` },
      { label: "Intel · .dmg", url: `${release}/Caper-macOS-Intel.dmg` },
    ],
  },
  {
    platform: "windows",
    label: "Windows",
    links: [{ label: "x64 installer · .exe", url: `${release}/Caper-Windows-x64-Setup.exe` }],
  },
  {
    platform: "linux",
    label: "Linux · x64",
    links: [
      { label: "Debian / Ubuntu · .deb", url: `${release}/Caper-Linux-x64.deb` },
      { label: "Portable archive · .tar.gz", url: `${release}/Caper-Linux-x64.tar.gz` },
    ],
    note: "Ubuntu 24.04 or compatible systems. The archive is for advanced use, not a universal installer.",
  },
  ...(mobileBetas.ios
    ? [{ platform: "ios" as const, label: "iOS", links: [{ label: "Join TestFlight", url: mobileBetas.ios.url }] }]
    : []),
  ...(mobileBetas.android
    ? [
        {
          platform: "android" as const,
          label: "Android",
          links: [
            { label: "1. Join testers group", url: mobileBetas.android.url },
            { label: "2. Install from Google Play", url: playTestingUrl },
          ],
          note: "Join the group with the Google account you use for Play, then install the beta.",
        },
      ]
    : []),
];

// Detection preselects the visitor's platform; every package stays available.
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
