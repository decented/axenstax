#!/usr/bin/env bash
# ci-ssh-keyscan.sh — resilient ssh-keyscan for CI.
#
# The bare `ssh-keyscan -H ... >> known_hosts 2>/dev/null` used in
# deploy.yml / publish-installers.yml fails silently and with no retry when
# the target box's sshd accepts the TCP connection but is slow (or, worse,
# never sends an SSH banner at all) — ssh-keyscan just times out after its
# 5s default and exits 1, and the swallowed stderr means the Actions log
# shows nothing but "Process completed with exit code 1". This script
# retries with a generous per-attempt timeout, and on failure prints a
# diagnosis that distinguishes "TCP refused/unreachable" from "TCP accepted
# but no SSH banner" so a human doesn't have to re-derive that from nothing.
#
# Usage: ci-ssh-keyscan.sh <host> [port]
#
# Env overrides (mainly for local/CI testing — defaults are the real values):
#   KEYSCAN_ATTEMPTS   number of attempts                  (default 6)
#   KEYSCAN_DELAY      seconds to sleep between attempts    (default 20)
#   KEYSCAN_TIMEOUT    ssh-keyscan -T value, seconds         (default 10)
#   KNOWN_HOSTS_FILE   override target file (default ~/.ssh/known_hosts)
set -euo pipefail

host="${1:?usage: ci-ssh-keyscan.sh <host> [port]}"
port="${2:-22}"

attempts="${KEYSCAN_ATTEMPTS:-6}"
delay="${KEYSCAN_DELAY:-20}"
keyscan_timeout="${KEYSCAN_TIMEOUT:-10}"

ssh_dir="$HOME/.ssh"
known_hosts="${KNOWN_HOSTS_FILE:-$ssh_dir/known_hosts}"
mkdir -p "$ssh_dir"
chmod 700 "$ssh_dir"
touch "$known_hosts"
chmod 600 "$known_hosts"

run_id="${GITHUB_RUN_ID:-<run id>}"

# Raw TCP reachability probe, distinct from "sshd answered with an SSH
# banner". bash's /dev/tcp pseudo-device is a shell builtin, always present
# on ubuntu-latest runners (unlike nc, which is present today but is an
# extra dependency we don't need — see also `which nc` check during
# development: OpenBSD netcat is installed on ubuntu-latest, but /dev/tcp
# needs nothing extra and works the same on any bash).
tcp_probe() {
  local h="$1" p="$2" err
  if err=$(timeout 5 bash -c "exec 3<>'/dev/tcp/${h}/${p}'" 2>&1); then
    echo "accepted"
  else
    # Collapse bash's (sometimes 2-line) connect error onto one line.
    err="$(printf '%s' "$err" | tr '\n' ' ' | sed 's/  */ /g; s/ $//')"
    echo "refused:${err}"
  fi
}

diagnose() {
  local probe
  probe="$(tcp_probe "$host" "$port")"
  if [[ "$probe" == accepted* ]]; then
    echo "diagnosis: TCP connect to ${host}:${port} succeeded, but ssh-keyscan got no SSH banner/keys back — sshd is not answering (HTTPS on the same box may still be up)."
  else
    echo "diagnosis: TCP connect to ${host}:${port} failed — ${probe#refused:} (connection refused or host unreachable)."
  fi
}

for attempt in $(seq 1 "$attempts"); do
  echo "ssh-keyscan attempt ${attempt}/${attempts} for ${host}:${port}..."

  err_file="$(mktemp)"
  rc=0
  out="$(ssh-keyscan -H -T "$keyscan_timeout" -p "$port" "$host" 2>"$err_file")" || rc=$?
  err="$(cat "$err_file")"
  rm -f "$err_file"

  # Some ssh-keyscan builds exit 0 even when they found nothing, so also
  # check the output actually contains key lines (non-comment, non-blank).
  key_lines="$(printf '%s\n' "$out" | grep -vc '^#\|^$' || true)"

  if [[ "$rc" -eq 0 && "$key_lines" -gt 0 ]]; then
    printf '%s\n' "$out" >> "$known_hosts"
    echo "ssh-keyscan succeeded on attempt ${attempt}: added ${key_lines} key line(s) to ${known_hosts}"
    exit 0
  fi

  echo "ssh-keyscan attempt ${attempt} found no keys (exit ${rc})." >&2
  if [[ -n "$err" ]]; then
    echo "ssh-keyscan stderr: ${err}" >&2
  fi

  if [[ "$attempt" -eq 1 ]]; then
    diagnose >&2
  fi

  if [[ "$attempt" -lt "$attempts" ]]; then
    echo "retrying in ${delay}s..." >&2
    sleep "$delay"
  fi
done

total_wait_sec=$(( (attempts - 1) * delay ))
wait_min=$(( (total_wait_sec + 59) / 60 ))
[[ "$wait_min" -lt 1 ]] && wait_min=1

echo "" >&2
diagnose >&2
final_probe="$(tcp_probe "$host" "$port")"
echo "" >&2
if [[ "$final_probe" == accepted* ]]; then
  echo "${host}:${port} accepted the TCP connection but sent no SSH banner after ${attempts} attempts over ~${wait_min} min — the box's sshd is not answering (HTTPS may still be up). Check the box console / sshd, then re-run this job: gh run rerun ${run_id} --failed" >&2
else
  echo "${host}:${port} refused the TCP connection or was unreachable after ${attempts} attempts over ~${wait_min} min (${final_probe#refused:}). Check the box's firewall / network, then re-run this job: gh run rerun ${run_id} --failed" >&2
fi
exit 1
