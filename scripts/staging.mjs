#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";

const profile = process.env.AWS_PROFILE?.trim() || "staging";
const region = "us-east-1";
const environment = { ...process.env };

function aws(args) {
  return spawnSync("aws", [...args, "--profile", profile, "--region", region], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

const credentialsResult = aws(["configure", "export-credentials", "--format", "process"]);
if (credentialsResult.status !== 0) {
  throw new Error(`AWS profile ${profile} is not authenticated; run: aws sso login --profile ${profile}`);
}
let credentials;
try {
  credentials = JSON.parse(credentialsResult.stdout);
} catch {
  throw new Error(`AWS profile ${profile} returned invalid temporary credentials`);
}
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
