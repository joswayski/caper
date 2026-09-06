import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { basename } from "node:path";
import { pathToFileURL } from "node:url";

const API_VERSION = "2022-11-28";
const REQUIRED_PLATFORMS = ["darwin-aarch64", "windows-x86_64", "linux-x86_64"];

function configuration() {
  const repository = process.env.GITHUB_REPOSITORY;
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (!repository || !/^[^/\s]+\/[^/\s]+$/u.test(repository)) {
    throw new Error("GITHUB_REPOSITORY must identify owner/name");
  }
  if (!token) throw new Error("GH_TOKEN or GITHUB_TOKEN is required");
  return { repository, token };
}

async function githubRequest(url, options = {}) {
  const { token } = configuration();
  const response = await fetch(url, {
    ...options,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": API_VERSION,
      ...options.headers,
    },
  });
  if (!response.ok) {
    const detail = (await response.text()).trim();
    throw new Error(`GitHub request failed (${response.status}): ${detail || response.statusText}`);
  }
  return response;
}

function apiUrl(path) {
  return `https://api.github.com/repos/${configuration().repository}/${path}`;
}

function assetNameFromUrl(url, assetsById) {
  const apiMatch = String(url).match(/\/releases\/assets\/(\d+)$/u);
  if (apiMatch) return assetsById.get(apiMatch[1])?.name;

  const downloadMatch = String(url).match(/\/releases\/download\/[^/]+\/([^/?#]+)$/u);
  return downloadMatch ? decodeURIComponent(downloadMatch[1]) : undefined;
}

export async function uploadReleaseAsset(releaseId, path) {
  const name = basename(path);
  const metadata = await stat(path);
  if (!metadata.isFile()) throw new Error(`release asset is not a file: ${path}`);

  const assetsResponse = await githubRequest(apiUrl(`releases/${releaseId}/assets?per_page=100`));
  const assets = await assetsResponse.json();
  for (const asset of assets.filter((candidate) => candidate.name === name)) {
    await githubRequest(apiUrl(`releases/assets/${asset.id}`), { method: "DELETE" });
  }

  const releaseResponse = await githubRequest(apiUrl(`releases/${releaseId}`));
  const release = await releaseResponse.json();
  const uploadUrl = release.upload_url?.replace(/\{\?.*$/u, "");
  if (!uploadUrl) throw new Error(`release ${releaseId} is missing its upload URL`);
  await githubRequest(`${uploadUrl}?name=${encodeURIComponent(name)}`, {
    method: "POST",
    headers: {
      "Content-Length": String(metadata.size),
      "Content-Type": "application/octet-stream",
    },
    body: createReadStream(path),
    duplex: "half",
  });
}

export async function finalizeRelease(releaseId, tag, appVersion) {
  const assetsResponse = await githubRequest(apiUrl(`releases/${releaseId}/assets?per_page=100`));
  const assets = await assetsResponse.json();
  const names = new Set(assets.map((asset) => asset.name));
  for (const [label, predicate] of [
    ["macOS DMG", (name) => name.endsWith(".dmg")],
    ["macOS updater", (name) => name.endsWith(".app.tar.gz")],
    ["macOS updater signature", (name) => name.endsWith(".app.tar.gz.sig")],
    ["Windows installer", (name) => /[-_]setup\.exe$/u.test(name)],
    ["Windows updater signature", (name) => name.endsWith(".exe.sig")],
    ["Linux AppImage", (name) => name.endsWith(".AppImage")],
    ["Linux updater signature", (name) => name.endsWith(".AppImage.sig")],
    ["Linux Debian package", (name) => name.endsWith(".deb")],
  ]) {
    if (![...names].some(predicate)) throw new Error(`release is missing its ${label}`);
  }

  const latestAsset = assets.find((asset) => asset.name === "latest.json");
  if (!latestAsset) throw new Error("release is missing latest.json");
  const latestResponse = await githubRequest(apiUrl(`releases/assets/${latestAsset.id}`), {
    headers: { Accept: "application/octet-stream" },
  });
  const latest = JSON.parse(await latestResponse.text());
  if (latest.version !== appVersion) {
    throw new Error(`latest.json version ${latest.version} does not match ${appVersion}`);
  }

  const assetsById = new Map(assets.map((asset) => [String(asset.id), asset]));
  for (const platform of REQUIRED_PLATFORMS) {
    const entry = latest.platforms?.[platform];
    if (!entry?.url || !entry?.signature) {
      throw new Error(`latest.json is missing a complete ${platform} entry`);
    }
    const name = assetNameFromUrl(entry.url, assetsById);
    if (!name || !names.has(name)) {
      throw new Error(`latest.json ${platform} does not identify an uploaded updater asset`);
    }
    entry.url = `https://github.com/${configuration().repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
  }

  await githubRequest(apiUrl(`releases/assets/${latestAsset.id}`), { method: "DELETE" });
  const releaseResponse = await githubRequest(apiUrl(`releases/${releaseId}`));
  const release = await releaseResponse.json();
  const uploadUrl = release.upload_url?.replace(/\{\?.*$/u, "");
  if (!uploadUrl) throw new Error(`release ${releaseId} is missing its upload URL`);
  await githubRequest(`${uploadUrl}?name=latest.json`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: `${JSON.stringify(latest, null, 2)}\n`,
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2);
  if (args[0] === "upload") {
    const [, releaseId, path] = args;
    if (!/^[1-9]\d*$/u.test(releaseId ?? "") || !path) {
      throw new Error("usage: node scripts/finalize-desktop-release.mjs upload <release-id> <path>");
    }
    await uploadReleaseAsset(releaseId, path);
    process.stdout.write(`Uploaded ${basename(path)} to release ${releaseId}.\n`);
    process.exit(0);
  }

  const [releaseId, tag, appVersion] = args;
  if (!/^[1-9]\d*$/u.test(releaseId ?? "") || !tag || !appVersion) {
    throw new Error("usage: node scripts/finalize-desktop-release.mjs <release-id> <tag> <app-version>");
  }
  await finalizeRelease(releaseId, tag, appVersion);
  process.stdout.write(`Validated release ${tag} and finalized latest.json.\n`);
}
