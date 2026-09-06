import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const RELEASE_TIME_ZONE = "America/New_York";

export function releaseDate(now = new Date(), timeZone = RELEASE_TIME_ZONE) {
  const fields = Object.fromEntries(
    new Intl.DateTimeFormat("en-US", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    })
      .formatToParts(now)
      .filter(({ type }) => type !== "literal")
      .map(({ type, value }) => [type, value]),
  );
  return `${fields.year}-${fields.month}-${fields.day}`;
}

export function nextReleaseVersion(date, tags) {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/u.exec(date);
  if (!match) throw new Error(`release date must use YYYY-MM-DD, received ${date}`);

  const [, yearText, monthText, dayText] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const normalized = new Date(`${date}T12:00:00Z`);
  if (
    Number.isNaN(normalized.valueOf())
    || normalized.getUTCFullYear() !== year
    || normalized.getUTCMonth() + 1 !== month
    || normalized.getUTCDate() !== day
  ) {
    throw new Error(`release date is not a real calendar date: ${date}`);
  }

  const prefix = `v${yearText}.${monthText}.${dayText}.`;
  const revisions = tags
    .filter((tag) => tag.startsWith(prefix))
    .map((tag) => {
      const suffix = tag.slice(prefix.length);
      if (!/^[1-9]\d*$/u.test(suffix)) {
        throw new Error(`malformed Caper release tag for ${date}: ${tag}`);
      }
      return Number(suffix);
    });
  const revision = Math.max(0, ...revisions) + 1;
  if (!Number.isSafeInteger(revision) || revision > 99) {
    throw new Error(`Caper already has 99 releases for ${date}`);
  }

  const displayVersion = `${yearText}.${monthText}.${dayText}.${revision}`;
  return {
    displayVersion,
    tag: `v${displayVersion}`,
    appVersion: `${year}.${month}.${day * 100 + revision}`,
  };
}

export function configuredReleaseDate(timestamp = process.env.CAPER_RELEASE_TIMESTAMP) {
  if (!timestamp) return releaseDate();
  const parsed = new Date(timestamp);
  if (Number.isNaN(parsed.valueOf())) {
    throw new Error(`CAPER_RELEASE_TIMESTAMP must be an ISO-8601 timestamp, received ${timestamp}`);
  }
  return releaseDate(parsed);
}

function main() {
  const tags = execFileSync("git", ["tag", "--list"], { encoding: "utf8" })
    .split(/\r?\n/u)
    .filter(Boolean);
  const version = nextReleaseVersion(configuredReleaseDate(), tags);
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(
      process.env.GITHUB_OUTPUT,
      [
        `display_version=${version.displayVersion}`,
        `tag=${version.tag}`,
        `app_version=${version.appVersion}`,
        "",
      ].join("\n"),
    );
  }
  process.stdout.write(`${JSON.stringify(version)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
