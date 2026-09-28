#!/usr/bin/env bash
# Rewrite the "Caper apps build is ready" Discord notification with the Native
# release result, mirroring joswayski/infrastructure's
# scripts/update-discord-deployment.sh. Godis turned the button into
# "Releasing…" when it dispatched this run; this restores the final controls.
set -euo pipefail

git_sha=""
message_id=""
result=""
run_url=""
started_at=""
finished_at=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --git-sha) git_sha="${2:-}"; shift 2 ;;
    --message-id) message_id="${2:-}"; shift 2 ;;
    --result) result="${2:-}"; shift 2 ;;
    --run-url) run_url="${2:-}"; shift 2 ;;
    --started-at) started_at="${2:-}"; shift 2 ;;
    --finished-at) finished_at="${2:-}"; shift 2 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ -n "${DEPLOY_NOTIFICATION_WEBHOOK_URL:-}" ]] || {
  echo "DEPLOY_NOTIFICATION_WEBHOOK_URL is required." >&2
  exit 1
}
webhook_url="${DEPLOY_NOTIFICATION_WEBHOOK_URL%%\?*}"
webhook_url="${webhook_url%/}"
[[ "$webhook_url" =~ ^https://discord\.com/api/webhooks/[0-9]+/[A-Za-z0-9._-]+$ ]] || {
  echo "DEPLOY_NOTIFICATION_WEBHOOK_URL must be a Discord webhook URL." >&2
  exit 1
}
[[ "$message_id" =~ ^[0-9]{17,20}$ ]] || { echo "message_id must be a Discord snowflake." >&2; exit 2; }
[[ "$git_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "git_sha must be a full lowercase 40-character Git SHA." >&2; exit 2; }
[[ "$run_url" =~ ^https://github\.com/joswayski/caper/actions/runs/[0-9]+$ ]] || {
  echo "run_url must identify a caper GitHub Actions run." >&2
  exit 2
}
[[ -z "$started_at" || "$started_at" =~ ^[0-9]{10}$ ]] || { echo "started_at must be empty or a Unix timestamp." >&2; exit 2; }
[[ "$finished_at" =~ ^[0-9]{10}$ ]] || { echo "finished_at must be a Unix timestamp." >&2; exit 2; }

short_sha="${git_sha:0:7}"
if [[ -n "$started_at" && "$finished_at" -ge "$started_at" ]]; then
  seconds=$((10#$finished_at - 10#$started_at))
  printf -v timing 'Started: <t:%s:T>\nEnded: <t:%s:T>\nDuration: %dm %ds' \
    "$started_at" "$finished_at" $((seconds / 60)) $((seconds % 60))
else
  timing="Ended: <t:${finished_at}:T>"
fi
releases="https://github.com/joswayski/caper/releases/tag/native-latest"
custom_id="production-deploy:v1:caper-apps:${git_sha}"

case "$result" in
  success)
    content="Release completed for **Caper apps** (\`${short_sha}\`): Mac, Windows, Linux and Android downloads are [updated](${releases}), and the iPhone build is processing in TestFlight.

${timing}

[View workflow run](${run_url})."
    label="Released"
    style=3
    disabled=true
    ;;
  failure)
    content="Release did not succeed for **Caper apps** (\`${short_sha}\`): **failure**.

${timing}

[View workflow run](${run_url}) and retry if safe."
    label="Retry release"
    style=3
    disabled=false
    ;;
  cancelled | skipped)
    content="Could not confirm the **Caper apps** release: **${result}**.

${timing}

[Check the workflow run](${run_url}) before taking another action."
    label="Check GitHub"
    style=2
    disabled=true
    ;;
  *) echo "Unsupported release result: $result" >&2; exit 2 ;;
esac

payload="$(jq -n \
  --arg content "$content" \
  --arg custom_id "$custom_id" \
  --arg label "$label" \
  --arg run_url "$run_url" \
  --argjson style "$style" \
  --argjson disabled "$disabled" \
  '{
    content: $content,
    allowed_mentions: {parse: []},
    components: [{type: 1, components: [
      {type: 2, style: $style, label: $label, custom_id: $custom_id, disabled: $disabled},
      {type: 2, style: 5, label: "Open GitHub", url: $run_url}
    ]}]
  }')"

curl --fail-with-body --silent --show-error --request PATCH \
  --header 'Content-Type: application/json' --data "$payload" \
  "${webhook_url}/messages/${message_id}" >/dev/null
echo "Updated the Caper apps Discord release message for run $run_url."
