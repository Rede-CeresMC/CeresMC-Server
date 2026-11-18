#!/usr/bin/env bash
# Runs the bot tests against a separate server instance (port 19140, copy of the worlds in
# target/test-run) so a server people are playing on is never touched.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=19140
RUN=target/test-run
stop_server() {
    # kill only the test copy; on Windows `kill` does not reach the native process
    if command -v taskkill >/dev/null 2>&1; then taskkill //IM ceres-test.exe //F >/dev/null 2>&1 || true
    else kill "${SERVER:-}" 2>/dev/null || true; fi
    sleep 0.5
}
stop_server
rm -rf "$RUN"
mkdir -p "$RUN"
if [ -f target/release/ceres.exe ]; then cp target/release/ceres.exe "$RUN/ceres-test.exe"; EXE=./ceres-test.exe
else cp target/release/ceres "$RUN/ceres-test"; EXE=./ceres-test; fi
cp -r data "$RUN/data"
if [ -d worlds ]; then cp -r worlds "$RUN/worlds"; fi
rm -f "$RUN"/worlds/*/db/LOCK
echo "BotOp" > "$RUN/ops.txt"
cat > "$RUN/server.properties" <<EOF
motd=CeresMC Test
server-port=$PORT
gamemode=creative
level-name=world
view-distance=4
EOF

(cd "$RUN" && exec "$EXE" > server.out 2>&1) &
SERVER=$!
trap stop_server EXIT
for _ in $(seq 1 100); do grep -q "Done" "$RUN/server.out" 2>/dev/null && break; sleep 0.1; done

export CERES_PORT=$PORT
status=0
python -I tools/smoke_client.py || status=1
python -I tools/blocks_test.py || status=1
python -I tools/inventory_test.py || status=1
python -I tools/commands_test.py || status=1

stop_server
(cd "$RUN" && exec "$EXE" > server2.out 2>&1) &
SERVER=$!
for _ in $(seq 1 100); do grep -q "Done" "$RUN/server2.out" 2>/dev/null && break; sleep 0.1; done
python -I tools/blocks_test.py check || status=1

if grep -E "bad packet|panicked" "$RUN"/server*.out; then status=1; fi
[ $status -eq 0 ] && echo "ALL TESTS PASSED" || echo "SOME TESTS FAILED"
exit $status
