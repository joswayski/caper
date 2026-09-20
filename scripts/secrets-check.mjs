#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const definitions = [
  ["DATABASE_URL", "required"],
  ["MIGRATION_DATABASE_URL", "required"],
  ["AUTH_SECRET", "required"],
  ["SES_FROM_ADDRESS", "required"],
  ["SES_CONFIGURATION_SET", "required"],
  ["AUTH_CODE_ATTEMPTS", "default: 3"],
  ["AUTH_EMAIL_15M_LIMIT", "default: 3"],
  ["AUTH_EMAIL_DAILY_LIMIT", "default: 5"],
  ["AUTH_IP_HOURLY_LIMIT", "default: 10"],
  ["AUTH_GLOBAL_HOURLY_LIMIT", "default: 500"],
  ["MEDIA_ENABLED", "default: false"],
  ["CF_SFU_APP_ID", "required when media is enabled"],
  ["CF_SFU_APP_SECRET", "required when media is enabled"],
  ["CF_TURN_KEY_ID", "required when media is enabled"],
  ["CF_TURN_API_TOKEN", "required when media is enabled"],
  ["AXIOM_TOKEN", "optional"],
  ["AXIOM_DATASET", "default: caper"],
  ["AXIOM_ENDPOINT", "required when Axiom is enabled"],
];

function option(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  if (index === -1) return fallback;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`--${name} requires a value`);
  return value;
}

function localEnvironment() {
  let source = "";
  try {
    source = readFileSync(".env", "utf8");
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const values = {};
  for (const line of source.split(/\r?\n/)) {
    const match = line.match(/^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)\s*$/);
    if (!match) continue;
    let value = match[2];
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    values[match[1]] = value;
  }
  return { ...values, ...process.env };
}

function run(command, args) {
  return spawnSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

const environment = option("environment", "staging");
const profile = option("profile", process.env.AWS_PROFILE?.trim() || environment);
const region = option("region", process.env.AWS_REGION?.trim() || "us-east-1");
const secretId = option("secret-id", `${environment}/apps/caper`);
const result = run("aws", [
  "secretsmanager", "get-secret-value",
  "--secret-id", secretId,
  "--query", "SecretString",
  "--output", "text",
  "--profile", profile,
  "--region", region,
]);
if (result.status !== 0) {
  throw new Error(`Could not read ${secretId} with AWS profile ${profile}. Authenticate with: aws sso login --profile ${profile}`);
}

let remote;
try {
  remote = JSON.parse(result.stdout);
} catch {
  throw new Error(`${secretId} does not contain a JSON object`);
}
if (!remote || Array.isArray(remote) || typeof remote !== "object") {
  throw new Error(`${secretId} does not contain a JSON object`);
}

const local = localEnvironment();
console.log(`Secrets Manager: ${secretId} (${profile}, ${region})`);
console.log("No secret values are printed.\n");
let incomplete = false;
for (const [key, requirement] of definitions) {
  const remoteValue = typeof remote[key] === "string" && remote[key].length > 0 ? remote[key] : undefined;
  const localValue = typeof local[key] === "string" && local[key].length > 0 ? local[key] : undefined;
  let status;
  if (remoteValue && localValue) status = remoteValue === localValue ? "in sync" : "differs (Secrets Manager wins)";
  else if (remoteValue) status = "Secrets Manager";
  else if (localValue) status = "fallback only";
  else status = `missing (${requirement})`;
  if (!remoteValue && requirement === "required") incomplete = true;
  console.log(`${key.padEnd(28)} ${status}`);
}

if (process.argv.includes("--cluster")) {
  console.log("\nExternalSecret status (values are not read):");
  for (const name of ["caper-api", "caper-api-account", "caper-api-database", "caper-api-axiom"]) {
    const status = run("kubectl", [
      "--namespace", "default", "get", "externalsecret", name,
      "--output", "jsonpath={.status.conditions[?(@.type=='Ready')].status}",
    ]);
    if (status.status === 0) console.log(`${name.padEnd(28)} ${status.stdout || "not ready"}`);
    else if (/not found/i.test(status.stderr)) console.log(`${name.padEnd(28)} not installed`);
    else console.log(`${name.padEnd(28)} unable to query current kubectl context`);
  }
}

if (incomplete) process.exitCode = 1;
