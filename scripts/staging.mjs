#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";

const profile = process.env.AWS_PROFILE?.trim() || "staging";
const region = "us-east-1";
const secretId = "staging/apps/caper";
const environment = { ...process.env };
const secretKeys = [
  "AUTH_SECRET",
  "MEDIA_ENABLED",
  "CF_SFU_APP_ID",
  "CF_SFU_APP_SECRET",
  "CF_TURN_KEY_ID",
  "CF_TURN_API_TOKEN",
  "AXIOM_TOKEN",
  "AXIOM_ENDPOINT",
];

function aws(args) {
  return spawnSync("aws", [...args, "--profile", profile, "--region", region], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

const secretResult = aws([
  "secretsmanager",
  "get-secret-value",
  "--secret-id",
  secretId,
  "--query",
  "SecretString",
  "--output",
  "text",
]);

if (secretResult.status === 0) {
  let secret;
  try {
    secret = JSON.parse(secretResult.stdout);
  } catch {
    throw new Error(`${secretId} must contain a JSON object`);
  }
  if (!secret || Array.isArray(secret) || typeof secret !== "object") {
    throw new Error(`${secretId} must contain a JSON object`);
  }
  for (const key of secretKeys) {
    if (typeof secret[key] === "string" && secret[key].length > 0) {
      environment[key] = secret[key];
    }
  }
  console.log(`Loaded staging application secrets from ${secretId}.`);
} else {
  console.warn(`${secretId} is unavailable; falling back to the repository .env file.`);
}

const credentialsResult = aws(["configure", "export-credentials", "--format", "process"]);
if (credentialsResult.status !== 0) {
  throw new Error(`AWS profile ${profile} is not authenticated; run: aws sso login --profile ${profile}`);
}
const credentials = JSON.parse(credentialsResult.stdout);
for (const [source, target] of [
  ["AccessKeyId", "AWS_ACCESS_KEY_ID"],
  ["SecretAccessKey", "AWS_SECRET_ACCESS_KEY"],
  ["SessionToken", "AWS_SESSION_TOKEN"],
]) {
  if (typeof credentials[source] !== "string" || credentials[source].length === 0) {
    throw new Error(`AWS profile ${profile} returned incomplete temporary credentials`);
  }
  environment[target] = credentials[source];
}
environment.AWS_EC2_METADATA_DISABLED = "true";

const compose = spawn(
  "docker",
  ["compose", "--file", "compose.staging.yaml", "up", "--build", ...process.argv.slice(2)],
  { env: environment, stdio: "inherit" },
);
compose.on("error", (error) => {
  console.error(`Unable to start Docker Compose: ${error.message}`);
  process.exitCode = 1;
});
compose.on("exit", (code, signal) => {
  process.exitCode = signal ? 1 : (code ?? 1);
});
