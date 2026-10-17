#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
temp_dir=$(mktemp -d)
entry_pid=
cleanup() {
    [ -z "$entry_pid" ] || kill -KILL "$entry_pid" 2>/dev/null || true
    # A killed supervisor cannot stop the stand-in daemons it started.
    if [ -f "$temp_dir/pids" ]; then
        while read -r pid; do
            [ "$(cat "/proc/$pid/comm" 2>/dev/null)" != sleep ] || kill -KILL "$pid" 2>/dev/null || true
        done < "$temp_dir/pids"
    fi
    rm -rf "$temp_dir"
}
trap cleanup EXIT HUP INT TERM
mkdir "$temp_dir/settings"
touch "$temp_dir/settings/settings.toml"
printf '%s\n' 'nsec1synthetic-test-only' > "$temp_dir/mostro.nsec"
chmod 600 "$temp_dir/mostro.nsec"

# Replace only the daemon executable in an isolated copy. No real Mostro or LND
# process is started by this test.
sed "s@/usr/local/bin/mostrod@$temp_dir/mostrod@g" docker/mostro-entrypoint.sh > "$temp_dir/entrypoint"
cat > "$temp_dir/mostrod" <<'SH'
#!/bin/sh
[ "$MOSTRO_NSEC_PRIVKEY" = nsec1synthetic-test-only ] || exit 1
[ "$1" = -d ] && [ "$2" = "$MOSTRO_SETTINGS_DIR" ] || exit 1
SH
chmod 700 "$temp_dir/mostrod"
/bin/sh -n "$temp_dir/entrypoint"

run_entrypoint() {
    MOSTRO_IDENTITY_FILE="$temp_dir/mostro.nsec" \
    MOSTRO_SETTINGS_DIR="$temp_dir/settings" \
    /bin/sh "$temp_dir/entrypoint" 2>/dev/null
}

# ── Direct mode: starts only with a safe identity and settings ───────────────
run_entrypoint
chmod 644 "$temp_dir/mostro.nsec"
if run_entrypoint; then exit 1; fi
chmod 600 "$temp_dir/mostro.nsec"
mv "$temp_dir/mostro.nsec" "$temp_dir/real.nsec"
ln -s "$temp_dir/real.nsec" "$temp_dir/mostro.nsec"
if run_entrypoint; then exit 1; fi
rm "$temp_dir/mostro.nsec"
mv "$temp_dir/real.nsec" "$temp_dir/mostro.nsec"
rm "$temp_dir/settings/settings.toml"
if run_entrypoint; then exit 1; fi
touch "$temp_dir/settings/settings.toml"
if MOSTRO_NSEC_PRIVKEY='nsec1docker-environment' run_entrypoint; then exit 1; fi
# An identity without a trailing newline is accepted; an empty one is refused
# with the reason in the log, not with a silent exit.
printf '%s' 'nsec1synthetic-test-only' > "$temp_dir/mostro.nsec"
run_entrypoint
: > "$temp_dir/mostro.nsec"
refusal=$(MOSTRO_IDENTITY_FILE="$temp_dir/mostro.nsec" MOSTRO_SETTINGS_DIR="$temp_dir/settings" /bin/sh "$temp_dir/entrypoint" 2>&1) && exit 1
case "$refusal" in
    *'private identity is not an nsec'*) ;;
    *) printf '%s\n' "empty identity refused without a reason: $refusal" >&2; exit 1 ;;
esac
printf '%s\n' 'nsec1synthetic-test-only' > "$temp_dir/mostro.nsec"

# ── Database copy before another mostrod version first starts ───────────────
snapshots="$temp_dir/db-snapshots"
marker="$temp_dir/settings/mostro.version"
start_version() {
    MOSTROD_VERSION=$1 MOSTRO_DB_SNAPSHOT_KEEP=${2:-5} run_entrypoint
}
copies() { ls "$snapshots"/mostro-*.db 2>/dev/null | wc -l; }
# A first start without a database records the version and copies nothing.
start_version 1.0.0
[ "$(cat "$marker")" = 1.0.0 ] || { echo 'version not recorded on a first start' >&2; exit 1; }
[ ! -e "$snapshots" ] || { echo 'a copy was made with no database' >&2; exit 1; }
# The same version again: nothing to copy.
printf 'datos 1.0.0' > "$temp_dir/settings/mostro.db"
start_version 1.0.0
[ ! -e "$snapshots" ] || { echo 'a copy was made without a version change' >&2; exit 1; }
# Another version: the database is copied as it was, in a private folder.
start_version 1.1.0
[ "$(copies)" -eq 1 ] || { echo 'no copy before the version change' >&2; exit 1; }
[ "$(cat "$snapshots"/mostro-1.0.0-a-1.1.0-*.db)" = 'datos 1.0.0' ] || { echo 'the copy is not the database before the change' >&2; exit 1; }
[ "$(stat -c %a "$snapshots")" = 700 ] || { echo 'the copies folder is not private' >&2; exit 1; }
[ "$(cat "$marker")" = 1.1.0 ] || { echo 'new version not recorded' >&2; exit 1; }
# An install from before this check (no marker) is copied too.
rm "$marker"
start_version 1.1.0
ls "$snapshots"/mostro-desconocida-a-1.1.0-*.db >/dev/null 2>&1 || { echo 'no copy for an install without a recorded version' >&2; exit 1; }
# Only the newest copies are kept.
start_version 1.2.0 2
start_version 1.3.0 2
[ "$(copies)" -eq 2 ] || { echo "kept $(copies) copies instead of 2" >&2; exit 1; }
# Without a place for the copy, mostrod does not start, and the version is not recorded.
rm -rf "$snapshots"
mkdir "$temp_dir/locked"
chmod 500 "$temp_dir/locked"
if MOSTROD_VERSION=2.0.0 MOSTRO_DB_SNAPSHOT_DIR="$temp_dir/locked/copias" run_entrypoint; then
    echo 'mostrod started without a copy of the database' >&2; exit 1
fi
[ "$(cat "$marker")" = 1.3.0 ] || { echo 'version recorded although no copy was made' >&2; exit 1; }
chmod 700 "$temp_dir/locked"
rm -f "$marker" "$temp_dir/settings/mostro.db"

# ── Standby mode: the supervisor loop used by the Umbrel package ─────────────
fail() {
    printf '%s\n' "entrypoint smoke failed: $1" >&2
    exit 1
}
wait_for() {
    # wait_for <seconds> <test expression...>
    remaining=$(($1 * 10))
    shift
    until "$@"; do
        remaining=$((remaining - 1))
        [ "$remaining" -gt 0 ] || return 1
        sleep 0.1
    done
}
standby_dir="$temp_dir/standby"
active="$standby_dir/active"
mkdir -p "$active"
start_standby() {
    STANDBY_IF_UNCONFIGURED=true \
    MOSTRO_IDENTITY_FILE="$temp_dir/mostro.nsec" \
    MOSTRO_SETTINGS_DIR="$active" \
    MOSTRO_RESTART_BACKOFF_SECONDS=1 \
    MOSTRO_STOP_GRACE_SECONDS=2 \
    /bin/sh "$temp_dir/entrypoint" >/dev/null 2>&1 &
    entry_pid=$!
}

# 1. A refused start ends with an error status, not a silent success that
#    would leave the container stopped under `restart: on-failure`.
touch "$active/settings.toml"
chmod 644 "$temp_dir/mostro.nsec"
start_standby
status=0
wait "$entry_pid" || status=$?
entry_pid=
[ "$status" -ne 0 ] || fail 'a refused start in standby exited with status 0'
[ ! -e "$active/mostro.pid" ] || fail 'pid file left after a refused start'
chmod 600 "$temp_dir/mostro.nsec"

# 2. A daemon that dies on its own is recorded and retried after a pause.
cat > "$temp_dir/mostrod" <<SH
#!/bin/sh
echo run >> "$temp_dir/runs"
exit 3
SH
rm -f "$active/settings.toml"
start_standby
sleep 0.3
[ ! -e "$temp_dir/runs" ] || fail 'the daemon started without settings'
touch "$active/settings.toml"
wait_for 5 test -s "$active/mostro.last_exit" || fail 'the exit of the daemon was not recorded'
read -r _ exit_code uptime < "$active/mostro.last_exit"
[ "$exit_code" = 3 ] || fail "recorded exit code $exit_code instead of 3"
[ "$uptime" -le 2 ] || fail "recorded uptime $uptime"
sleep 2.5
runs=$(wc -l < "$temp_dir/runs")
[ "$runs" -ge 2 ] || fail 'the daemon was not restarted'
[ "$runs" -le 4 ] || fail "the daemon was restarted $runs times in under 4 s: no backoff"
kill -TERM "$entry_pid"
status=0
wait "$entry_pid" || status=$?
entry_pid=
[ "$status" -eq 0 ] || fail "a stop request ended with status $status"

# 3. A running daemon gets a heartbeat, is restarted on a wake request and is
#    stopped when the settings are withdrawn.
cat > "$temp_dir/mostrod" <<SH
#!/bin/sh
echo "\$\$" >> "$temp_dir/pids"
exec sleep 300
SH
rm -f "$temp_dir/runs" "$active/mostro.last_exit"
start_standby
wait_for 5 test -s "$active/mostro.pid" || fail 'no pid file for a running daemon'
wait_for 5 test -e "$active/mostro.heartbeat" || fail 'no heartbeat for a running daemon'
first=$(cat "$active/mostro.pid")
kill -0 "$first" 2>/dev/null || fail 'the recorded pid is not running'
# The pid file is written once per start and left alone: the panel reads its
# age as the time the daemon has been running. Only the heartbeat moves on.
started=$(stat -c %Y "$active/mostro.pid")
sleep 2.2
[ "$(stat -c %Y "$active/mostro.pid")" = "$started" ] || fail 'the pid file was touched after the daemon started'
[ "$(stat -c %Y "$active/mostro.heartbeat")" -gt "$started" ] || fail 'the heartbeat stopped moving'
touch "$standby_dir/.standby_wake"
# Between the two daemons the pid file does not exist: only a new, non-empty
# pid counts as a restart.
wait_for 8 sh -c "pid=\$(cat '$active/mostro.pid' 2>/dev/null) && [ -n \"\$pid\" ] && [ \"\$pid\" != '$first' ]" || fail 'the wake request did not restart the daemon'
! kill -0 "$first" 2>/dev/null || fail 'the previous daemon survived the restart'
[ ! -e "$standby_dir/.standby_wake" ] || fail 'the wake request was not consumed'
[ ! -e "$active/mostro.last_exit" ] || fail 'a requested restart was recorded as a crash'
second=$(cat "$active/mostro.pid")
# The panel signals the daemon itself when both share a process namespace. A
# daemon found dead with a restart pending was asked to stop: not a crash.
touch "$standby_dir/.standby_wake"
# The supervisor may have stopped it already on seeing the request.
kill -TERM "$second" 2>/dev/null || true
wait_for 8 sh -c "pid=\$(cat '$active/mostro.pid' 2>/dev/null) && [ -n \"\$pid\" ] && [ \"\$pid\" != '$second' ]" || fail 'a daemon stopped by the panel was not started again'
[ ! -e "$active/mostro.last_exit" ] || fail 'a stop the panel asked for was recorded as a crash'
third=$(cat "$active/mostro.pid")
rm "$active/settings.toml"
wait_for 8 sh -c "! kill -0 '$third' 2>/dev/null" || fail 'the daemon kept running without settings'
wait_for 5 sh -c "[ ! -e '$active/mostro.pid' ] && [ ! -e '$active/mostro.heartbeat' ]" || fail 'runtime markers left after deactivation'
# The standby loop writes this file on every turn and removes it when a daemon
# starts, so it proves the supervisor went back to waiting rather than being
# on its way out.
wait_for 5 test -s "$standby_dir/.mostro_standby.pid.sleep" || fail 'the supervisor did not return to standby'
kill -0 "$entry_pid" 2>/dev/null || fail 'the supervisor exited instead of returning to standby'
# The same when the panel withdraws the settings and stops the daemon itself.
touch "$active/settings.toml"
wait_for 8 test -s "$active/mostro.pid" || fail 'the daemon did not start again with new settings'
fourth=$(cat "$active/mostro.pid")
rm "$active/settings.toml"
kill -TERM "$fourth" 2>/dev/null || true
wait_for 8 sh -c "! kill -0 '$fourth' 2>/dev/null" || fail 'the daemon stopped by the panel is still running'
wait_for 5 test -s "$standby_dir/.mostro_standby.pid.sleep" || fail 'the supervisor did not return to standby after the panel stopped the daemon'
[ ! -e "$active/mostro.last_exit" ] || fail 'a deactivation by the panel was recorded as a crash'
[ ! -e "$active/mostro.pid" ] || fail 'pid file left after a deactivation by the panel'
kill -TERM "$entry_pid"
wait "$entry_pid" || true
entry_pid=

echo 'Mostro entrypoint smoke passed'
