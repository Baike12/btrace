#!/bin/sh
# Supervises the two processes this image runs.
#
# tini is PID 1 (see the Dockerfile ENTRYPOINT) and reaps zombies; this script
# only has to start both children, forward shutdown, and fail the container when
# either one dies. A half-up instance — UI serving while the API is gone — is
# worse than a restart, because the UI's /api/public/* proxy would 502 while the
# page itself still rendered.
set -eu

# The bind address is set explicitly rather than inherited: Docker sets HOSTNAME
# to the container id, and the backend must not try to bind to that.
#
# Both ports are passed per-command rather than exported. `PORT` is read by the
# Rust backend *and* by Next.js, so exporting it would make the web server try to
# bind the API's port and die with EADDRINUSE.
: "${LANGFUSE_BIND_ADDRESS:=0.0.0.0}"
: "${PORT:=8010}"
: "${WEB_PORT:=3000}"

echo "starting langfuse-server on ${LANGFUSE_BIND_ADDRESS}:${PORT}"
LANGFUSE_BIND_ADDRESS="$LANGFUSE_BIND_ADDRESS" PORT="$PORT" langfuse-server &
API_PID=$!

echo "starting web (next standalone) on :${WEB_PORT}"
PORT="$WEB_PORT" node web/server.js &
WEB_PID=$!

shutdown() {
    # TERM first so the API drains in-flight requests; the backend also stops its
    # queue consumers on this signal.
    kill -TERM "$API_PID" "$WEB_PID" 2>/dev/null || true
}

trap 'shutdown; exit 0' INT TERM

# `wait -n` is not available in POSIX sh, so poll for either child exiting.
while kill -0 "$API_PID" 2>/dev/null && kill -0 "$WEB_PID" 2>/dev/null; do
    sleep 1
done

echo "a child process exited; stopping the container" >&2
shutdown
wait 2>/dev/null || true
exit 1
