#!/usr/bin/env bash
# Roll every Caper component forward to its newest successful build, in order:
# API, chat gateway, web, then the native apps. Run by
# .github/workflows/deploy-production.yml after each image or native build.
#
# What is live is recorded as GitHub Deployments in this repository (one
# environment per component), so re-runs only deploy what is behind. Server
# components roll out through joswayski/infrastructure's deploy workflows,
# which edit the Discord message posted here with the result. The apps are
# released by release.yml, which does the same. The first failure stops the
# chain, so a later component never ships ahead of one it depends on.
#
# SEED=1 records each component's newest build as deployed without deploying
# anything: run it once when turning automatic deploys on, while production
# already runs those builds.
#
# Environment: GH_TOKEN (this repository: actions + deployments write),
# INFRA_TOKEN (joswayski/infrastructure: actions write),
# DEPLOY_NOTIFICATION_WEBHOOK_URL (optional; Discord status messages).
set -euo pipefail

repo="${GITHUB_REPOSITORY:-joswayski/caper}"
infra="joswayski/infrastructure"
: "${GH_TOKEN:?}" "${INFRA_TOKEN:?}"

# name | build workflow | GitHub environment | deploy target | display name
components=(
  "caper-api|api-image.yml|production-api|infra:deploy-caper-api.yml|Caper API"
  "caper-gateway|api-image.yml|production-gateway|infra:deploy-caper-gateway.yml|Caper chat gateway"
  "caper-web|aws-image.yml|production-web|infra:deploy-caper-web.yml|Caper web"
  "caper-apps|native.yml|production-apps|caper:release.yml|Caper apps"
)

newest_build() { # workflow -> newest SHA on main whose build succeeded
  gh api "repos/$repo/actions/workflows/$1/runs?branch=main&status=success&per_page=1" \
    --jq '.workflow_runs[0].head_sha // ""'
}

deployed_sha() { # environment -> SHA of the newest successful deployment
  local id
  for id in $(gh api "repos/$repo/deployments?environment=$1&per_page=20" --jq '.[].id'); do
    if [[ "$(gh api "repos/$repo/deployments/$id/statuses?per_page=1" --jq '.[0].state // ""')" == success ]]; then
      gh api "repos/$repo/deployments/$id" --jq .sha
      return
    fi
  done
}

is_newer() { # base head -> true when head is strictly ahead of base on main
  [[ -z "$1" ]] && return 0
  [[ "$(gh api "repos/$repo/compare/$1...$2" --jq .status)" == ahead ]]
}

commit_title() {
  gh api "repos/$repo/commits/$1" --jq '.commit.message | split("\n")[0]' 2>/dev/null || echo "$1"
}

post_status() { # display sha -> Discord message ID (empty without a webhook)
  [[ -n "${DEPLOY_NOTIFICATION_WEBHOOK_URL:-}" ]] || return 0
  local payload
  payload="$(jq -cn --arg name "$1" --arg sha "$2" --arg title "$(commit_title "$2")" \
    --arg run "${GITHUB_SERVER_URL:-https://github.com}/$repo/actions/runs/${GITHUB_RUN_ID:-}" '{
      username: "Production deploys",
      embeds: [{
        author: {name: ($name + " is deploying automatically")},
        title: ($title | if length > 256 then .[:253] + "..." else . end),
        color: 3447003,
        fields: [{name: "Git SHA", value: ("`" + $sha + "`"), inline: false}],
        url: $run
      }]
    }')"
  curl --fail --show-error --silent --retry 3 --retry-all-errors --max-time 10 \
    --header 'Content-Type: application/json' --data "$payload" \
    "${DEPLOY_NOTIFICATION_WEBHOOK_URL%%\?*}?wait=true" | jq -r .id
}

record() { # environment sha state [deployment id] -> deployment id
  local id="${4:-}"
  if [[ -z "$id" ]]; then
    id="$(jq -n --arg ref "$2" --arg env "$1" \
      '{ref: $ref, environment: $env, auto_merge: false, required_contexts: [], description: "Production deploy"}' |
      gh api -X POST "repos/$repo/deployments" --input - --jq .id)"
  fi
  gh api -X POST "repos/$repo/deployments/$id/statuses" -f state="$3" \
    -f log_url="${GITHUB_SERVER_URL:-https://github.com}/$repo/actions/runs/${GITHUB_RUN_ID:-}" >/dev/null
  echo "$id"
}

wait_for_run() { # token repository workflow sha since -> conclusion
  local token="$1" target="$2" workflow="$3" sha="$4" since="$5" run="" conclusion=""
  for _ in $(seq 60); do # find the dispatched run (its run-name carries the SHA)
    run="$(GH_TOKEN="$token" gh api "repos/$target/actions/workflows/$workflow/runs?event=workflow_dispatch&created=%3E%3D$since&per_page=20" \
      --jq "[.workflow_runs[] | select((.display_title | contains(\"$sha\")) or .head_sha == \"$sha\")][0].id // \"\"")"
    [[ -n "$run" ]] && break
    sleep 5
  done
  [[ -n "$run" ]] || { echo "No $workflow run appeared for $sha." >&2; echo missing; return; }
  echo "Waiting for $target run $run" >&2
  while :; do
    conclusion="$(GH_TOKEN="$token" gh api "repos/$target/actions/runs/$run" --jq 'if .status == "completed" then .conclusion else "" end')"
    [[ -n "$conclusion" ]] && { echo "$conclusion"; return; }
    sleep 15
  done
}

for entry in "${components[@]}"; do
  IFS="|" read -r _ build environment target display <<<"$entry"
  desired="$(newest_build "$build")"
  [[ -n "$desired" ]] || { echo "$display: no successful build yet."; continue; }
  current="$(deployed_sha "$environment" || true)"
  if ! is_newer "$current" "$desired"; then
    echo "$display: $current is current."
    continue
  fi
  if [[ "${SEED:-0}" == 1 ]]; then
    record "$environment" "$desired" success >/dev/null
    echo "$display: recorded $desired as deployed (seed)."
    continue
  fi
  echo "$display: deploying $desired (was ${current:-unknown})."
  message_id="$(post_status "$display" "$desired" || true)"
  started_at="$(date +%s)"
  since="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  deployment="$(record "$environment" "$desired" in_progress)"
  args=(-f git_sha="$desired" -f discord_started_at="$started_at")
  [[ -n "$message_id" ]] && args+=(-f discord_message_id="$message_id")

  if [[ "$target" == caper:* ]]; then
    # Apps only move forward and release.yml reports its own result, so
    # dispatching it is the deployment; it is not awaited.
    gh workflow run "${target#caper:}" -R "$repo" --ref main "${args[@]}"
    record "$environment" "$desired" success "$deployment" >/dev/null
    continue
  fi

  GH_TOKEN="$INFRA_TOKEN" gh workflow run "${target#infra:}" -R "$infra" --ref main "${args[@]}"
  conclusion="$(wait_for_run "$INFRA_TOKEN" "$infra" "${target#infra:}" "$desired" "$since")"
  if [[ "$conclusion" == success ]]; then
    record "$environment" "$desired" success "$deployment" >/dev/null
    echo "$display: deployed $desired."
  else
    record "$environment" "$desired" failure "$deployment" >/dev/null
    echo "::error::$display deploy of $desired ended with ${conclusion:-no result}; later components were not deployed."
    exit 1
  fi
done
