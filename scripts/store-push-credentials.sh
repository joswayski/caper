#!/usr/bin/env bash
# Merge the APNs and FCM push credentials into the API's Secrets Manager secret
# (production/apps/caper, or staging/apps/caper with --staging), keeping every
# other field. Secret values are never printed: key files are read by jq
# directly (--rawfile), so they never appear in arguments or output.
#
#   scripts/store-push-credentials.sh \
#     --team-id ABCDE12345 \
#     --apns-key-id KEY1234567 --apns-key ~/Downloads/AuthKey_KEY1234567.p8 \
#     --apns-sandbox-key-id KEY7654321 --apns-sandbox-key ~/Downloads/AuthKey_KEY7654321.p8 \
#     --fcm-service-account ~/Downloads/caper-push.json \
#     --platforms apns,apnsSandbox,fcm --enabled true \
#     --dry-run
#
# Every input is optional, so one platform can be added or rotated at a time.
# --dry-run lists only the field names that would change. Uses the AWS CLI's
# usual credentials and region (AWS_PROFILE, AWS_REGION).
set -euo pipefail

usage() {
  awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
}

secret_id="production/apps/caper"
dry_run=false
team_id=""
apns_key_id=""
apns_key=""
sandbox_key_id=""
sandbox_key=""
fcm_service_account=""
topic=""
platforms=""
enabled=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --staging) secret_id="staging/apps/caper"; shift ;;
    --dry-run) dry_run=true; shift ;;
    --team-id) team_id="${2:-}"; shift 2 ;;
    --apns-key-id) apns_key_id="${2:-}"; shift 2 ;;
    --apns-key) apns_key="${2:-}"; shift 2 ;;
    --apns-sandbox-key-id) sandbox_key_id="${2:-}"; shift 2 ;;
    --apns-sandbox-key) sandbox_key="${2:-}"; shift 2 ;;
    --fcm-service-account) fcm_service_account="${2:-}"; shift 2 ;;
    --topic) topic="${2:-}"; shift 2 ;;
    --platforms) platforms="${2:-}"; shift 2 ;;
    --enabled) enabled="${2:-}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "Unknown argument: $1 (see --help)" >&2; exit 2 ;;
  esac
done

fail() { echo "$1" >&2; exit 2; }
command -v jq >/dev/null || fail "jq is required."
command -v aws >/dev/null || fail "The AWS CLI is required."

# Apple team and key IDs are 10 uppercase letters or digits.
for pair in "--team-id:$team_id" "--apns-key-id:$apns_key_id" "--apns-sandbox-key-id:$sandbox_key_id"; do
  value="${pair#*:}"
  [[ -z "$value" || "$value" =~ ^[A-Z0-9]{10}$ ]] || fail "${pair%%:*} must be 10 uppercase letters or digits."
done
check_key() {
  local option="$1" path="$2"
  [[ -f "$path" && -r "$path" ]] || fail "$option must be a readable .p8 file."
  # A .p8 is a bare PEM PKCS#8 key: its first line is the BEGIN marker.
  [[ "$(head -n 1 "$path" | tr -d '\r')" == "-----BEGIN PRIVATE KEY-----" ]] \
    && grep -q -- "^-----END PRIVATE KEY-----" "$path" \
    || fail "$option is not an APNs .p8 key (PEM PRIVATE KEY)."
}
if [[ -n "$apns_key_id$apns_key" ]]; then
  [[ -n "$apns_key_id" && -n "$apns_key" ]] || fail "--apns-key-id and --apns-key go together."
  check_key --apns-key "$apns_key"
fi
if [[ -n "$sandbox_key_id$sandbox_key" ]]; then
  [[ -n "$sandbox_key_id" && -n "$sandbox_key" ]] || fail "--apns-sandbox-key-id and --apns-sandbox-key go together."
  check_key --apns-sandbox-key "$sandbox_key"
fi
if [[ -n "$fcm_service_account" ]]; then
  [[ -f "$fcm_service_account" && -r "$fcm_service_account" ]] || fail "--fcm-service-account must be a readable JSON file."
  jq -e '.type == "service_account" and (.project_id | type == "string") and (.client_email | type == "string")
         and (.private_key | type == "string" and startswith("-----BEGIN PRIVATE KEY-----"))' \
    "$fcm_service_account" >/dev/null 2>&1 || fail "--fcm-service-account is not a Firebase service account JSON key."
fi
[[ -z "$topic" || "$topic" =~ ^[A-Za-z0-9.-]+$ ]] || fail "--topic must be a bundle ID."
[[ -z "$platforms" || "$platforms" =~ ^(apns|apnsSandbox|fcm)(,(apns|apnsSandbox|fcm))*$ ]] \
  || fail "--platforms must be a comma-separated list of apns, apnsSandbox and fcm (no spaces)."
[[ -z "$enabled" || "$enabled" == true || "$enabled" == false ]] || fail "--enabled must be true or false."
if [[ -n "$apns_key$sandbox_key" && -z "$team_id" ]]; then
  echo "Note: no --team-id given; the secret must already hold APNS_TEAM_ID." >&2
fi

# The new fields, built by jq from the files themselves. The FCM key is stored
# as one compact JSON string; .p8 keys keep their real newlines.
updates="$(jq -n \
  --arg team_id "$team_id" \
  --arg apns_key_id "$apns_key_id" \
  --rawfile apns_key "${apns_key:-/dev/null}" \
  --arg sandbox_key_id "$sandbox_key_id" \
  --rawfile sandbox_key "${sandbox_key:-/dev/null}" \
  --rawfile fcm "${fcm_service_account:-/dev/null}" \
  --arg topic "$topic" \
  --arg platforms "$platforms" \
  --arg enabled "$enabled" \
  '{
     APNS_TEAM_ID: $team_id,
     APNS_KEY_ID: $apns_key_id,
     APNS_PRIVATE_KEY: $apns_key,
     APNS_SANDBOX_KEY_ID: $sandbox_key_id,
     APNS_SANDBOX_PRIVATE_KEY: $sandbox_key,
     FCM_SERVICE_ACCOUNT_JSON: (if $fcm == "" then "" else ($fcm | fromjson | tojson) end),
     APNS_TOPIC: $topic,
     PUSH_PLATFORMS: $platforms,
     NOTIFICATIONS_ENABLED: $enabled
   } | with_entries(select(.value != ""))')"
[[ "$updates" != "{}" ]] || fail "Nothing to store: pass at least one credential or setting (see --help)."

current="$(aws secretsmanager get-secret-value --secret-id "$secret_id" --query SecretString --output text)" \
  || { echo "Could not read $secret_id." >&2; exit 1; }
jq -e 'type == "object"' >/dev/null 2>&1 <<<"$current" || { echo "$secret_id is not a JSON object." >&2; exit 1; }

# Field names only: which of the new values differ from what is stored.
changed="$(printf '%s\n%s\n' "$current" "$updates" \
  | jq -rs '.[0] as $old | .[1] | to_entries[] | select($old[.key] != .value) | .key')"
if [[ -z "$changed" ]]; then
  echo "$secret_id already holds these values; nothing to change."
  exit 0
fi
fields="$(paste -sd ' ' <<<"$changed")"
if [[ "$dry_run" == true ]]; then
  echo "Dry run: would update $secret_id fields: $fields"
  exit 0
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
chmod 700 "$workdir"
(umask 077 && printf '%s\n%s\n' "$current" "$updates" | jq -s '.[0] + .[1]' >"$workdir/secret.json")
version="$(aws secretsmanager put-secret-value --secret-id "$secret_id" \
  --secret-string "file://$workdir/secret.json" --query VersionId --output text)"
echo "Updated $secret_id fields: $fields (version $version)."
echo "The API reads the new values within about 5 minutes, when its ExternalSecret refreshes and Reloader restarts it."
