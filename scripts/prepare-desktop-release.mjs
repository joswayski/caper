import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const [appVersion] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+$/u.test(appVersion ?? "")) {
  throw new Error("usage: node scripts/prepare-desktop-release.mjs <app-version>");
}

const required = [
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  "TAURI_UPDATER_PUBLIC_KEY",
];
if (process.platform === "darwin") {
  required.push(
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "KEYCHAIN_PASSWORD",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY",
    "APPLE_API_PRIVATE_KEY",
  );
}
const missing = required.filter((name) => !process.env[name]);
if (missing.length > 0) {
  throw new Error(`missing release configuration: ${missing.join(", ")}`);
}

const base = JSON.parse(
  readFileSync(resolve("apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
);
const releaseConfiguration = {
  ...base,
  version: appVersion,
  bundle: {
    ...base.bundle,
    createUpdaterArtifacts: true,
  },
  plugins: {
    updater: {
      pubkey: process.env.TAURI_UPDATER_PUBLIC_KEY,
      endpoints: [
        "https://github.com/joswayski/caper/releases/download/preview/latest.json",
      ],
      windows: {
        installMode: "passive",
      },
    },
  },
};
writeFileSync(
  resolve("apps/desktop/src-tauri/tauri.release.conf.json"),
  `${JSON.stringify(releaseConfiguration, null, 2)}\n`,
);

if (process.platform === "darwin") {
  const runnerTemp = process.env.RUNNER_TEMP;
  const githubEnv = process.env.GITHUB_ENV;
  if (!runnerTemp || !githubEnv) {
    throw new Error("RUNNER_TEMP and GITHUB_ENV are required on macOS CI");
  }

  const certificatePath = join(runnerTemp, "caper-developer-id.p12");
  const apiKeyPath = join(runnerTemp, `AuthKey_${process.env.APPLE_API_KEY}.p8`);
  writeFileSync(certificatePath, Buffer.from(process.env.APPLE_CERTIFICATE, "base64"), {
    mode: 0o600,
  });
  writeFileSync(apiKeyPath, process.env.APPLE_API_PRIVATE_KEY, { mode: 0o600 });
  appendFileSync(
    githubEnv,
    [
      `APPLE_CERTIFICATE_PATH=${certificatePath}`,
      `APPLE_API_ISSUER=${process.env.APPLE_API_ISSUER}`,
      `APPLE_API_KEY=${process.env.APPLE_API_KEY}`,
      `APPLE_API_KEY_PATH=${apiKeyPath}`,
      "",
    ].join("\n"),
  );
}
