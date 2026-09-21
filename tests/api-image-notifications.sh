#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

awk '
  /^      - name: Notify deployment webhook$/ { step = 1 }
  step && /^        run: \|$/ { capture = 1; next }
  capture && /^      - / { exit }
  capture && /^          / { sub(/^          /, ""); print; next }
  capture && /^$/ { print; next }
  capture { exit 1 }
' "$repo_root/.github/workflows/api-image.yml" >"$tmp_dir/notify.sh"
[[ -s "$tmp_dir/notify.sh" ]]

cat >"$tmp_dir/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

payload=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --data)
      payload="$2"
      shift 2
      ;;
    --header)
      shift 2
      ;;
    --fail | --show-error | --silent | --retry-all-errors)
      shift
      ;;
    --retry | --max-time)
      shift 2
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

if [[ "$url" == */commits/*/pulls?per_page=1 ]]; then
  printf '[]\n'
elif [[ "$url" == https://discord.com/api/webhooks/* ]]; then
  count=0
  [[ ! -f "$FAKE_CURL_COUNT" ]] || count="$(cat "$FAKE_CURL_COUNT")"
  count=$((count + 1))
  printf '%s' "$count" >"$FAKE_CURL_COUNT"
  printf '%s' "$payload" >"$FAKE_CURL_PAYLOAD_PREFIX.$count.json"
  printf '%s' "$url" >"$FAKE_CURL_URL_PREFIX.$count"
else
  echo "Unexpected curl URL: $url" >&2
  exit 1
fi
EOF
chmod +x "$tmp_dir/curl" "$tmp_dir/notify.sh"

export PATH="$tmp_dir:$PATH"
export FAKE_CURL_COUNT="$tmp_dir/count"
export FAKE_CURL_PAYLOAD_PREFIX="$tmp_dir/payload"
export FAKE_CURL_URL_PREFIX="$tmp_dir/url"
export COMMIT_MESSAGE=$'Add gateway notifications\n\nKeep API deploys independent.'
export DEPLOY_NOTIFICATION_WEBHOOK_URL="https://discord.com/api/webhooks/123/token"
export DEPLOY_WORKFLOW_URL="https://github.com/joswayski/infrastructure/actions/workflows/deploy-caper-api.yml"
export GITHUB_API_URL="https://api.github.com"
export GITHUB_REPOSITORY="joswayski/caper"
export GITHUB_SHA="0123456789abcdef0123456789abcdef01234567"
export GITHUB_TOKEN="test-token"
export IMAGE_DIGEST="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

"$tmp_dir/notify.sh"

[[ "$(cat "$FAKE_CURL_COUNT")" == 2 ]]
for index in 1 2; do
  [[ "$(cat "$FAKE_CURL_URL_PREFIX.$index")" == \
    "$DEPLOY_NOTIFICATION_WEBHOOK_URL?wait=false&with_components=true" ]]
done

jq -e --arg sha "$GITHUB_SHA" --arg digest "$IMAGE_DIGEST" '
  .embeds[0].author.name == "Caper API image is ready" and
  .embeds[0].title == "Add gateway notifications" and
  .embeds[0].fields == [
    {name: "Description", value: "Keep API deploys independent.", inline: false},
    {name: "Git SHA", value: ("`" + $sha + "`"), inline: false},
    {name: "Digest", value: ("`" + $digest + "`"), inline: false}
  ] and
  .components[0].components[0] == {
    type: 2,
    style: 3,
    label: "Deploy Caper API",
    custom_id: ("production-deploy:v1:caper-api:" + $sha)
  } and
  .components[0].components[1].url == "https://github.com/joswayski/infrastructure/actions/workflows/deploy-caper-api.yml"
' "$FAKE_CURL_PAYLOAD_PREFIX.1.json" >/dev/null

jq -e --arg sha "$GITHUB_SHA" --arg digest "$IMAGE_DIGEST" '
  .embeds[0].author.name == "Caper chat gateway image is ready" and
  .embeds[0].fields[0] == {
    name: "Deploy API first",
    value: "Deploy the matching Caper API image successfully first. Gateway deployment verifies the live API rollout before changing its image.",
    inline: false
  } and
  .embeds[0].fields[-1].value == ("`" + $digest + "`") and
  .components[0].components[0] == {
    type: 2,
    style: 3,
    label: "Deploy Caper chat gateway",
    custom_id: ("production-deploy:v1:caper-gateway:" + $sha)
  } and
  .components[0].components[1].url == "https://github.com/joswayski/infrastructure/actions/workflows/deploy-caper-gateway.yml"
' "$FAKE_CURL_PAYLOAD_PREFIX.2.json" >/dev/null

echo "API image deployment notification payload tests passed."
