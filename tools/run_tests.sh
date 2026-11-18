#!/usr/bin/env bash
# Runs the bot tests against a separate server instance (port 19140, its own fresh flat world in
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
# CERES_BUILD=<folder> forces a build folder (e.g. target/work while target/release is locked by a running server)
if [ -n "${CERES_BUILD:-}" ]; then
    if [ -f "$CERES_BUILD/ceres.exe" ]; then cp "$CERES_BUILD/ceres.exe" "$RUN/ceres-test.exe"; EXE=./ceres-test.exe
    else cp "$CERES_BUILD/ceres" "$RUN/ceres-test"; EXE=./ceres-test; fi
elif [ -f target/debug/ceres.exe ] && { [ ! -f target/release/ceres.exe ] || [ target/debug/ceres.exe -nt target/release/ceres.exe ]; }; then cp target/debug/ceres.exe "$RUN/ceres-test.exe"; EXE=./ceres-test.exe
elif [ -f target/release/ceres.exe ]; then cp target/release/ceres.exe "$RUN/ceres-test.exe"; EXE=./ceres-test.exe
elif [ -f target/debug/ceres ] && { [ ! -f target/release/ceres ] || [ target/debug/ceres -nt target/release/ceres ]; }; then cp target/debug/ceres "$RUN/ceres-test"; EXE=./ceres-test
else cp target/release/ceres "$RUN/ceres-test"; EXE=./ceres-test; fi
cp -r data "$RUN/data"
# a resource pack (a folder, zipped by the server) big enough to need several chunks
mkdir -p "$RUN/resource_packs/TestPack/textures"
cat > "$RUN/resource_packs/TestPack/manifest.json" <<'EOF2'
{
  // comments are allowed in manifests
  "format_version": 2,
  "header": {"name": "Ceres Test Pack", "uuid": "5d7f8a3c-1b2e-4c6d-9e8f-0a1b2c3d4e5f", "version": [1, 0, 0], "min_engine_version": [1, 21, 0]},
  "modules": [{"type": "resources", "uuid": "6e8f9b4d-2c3f-4d7e-8f9a-1b2c3d4e5f60", "version": [1, 0, 0]}]
}
EOF2
head -c 600000 /dev/urandom > "$RUN/resource_packs/TestPack/textures/noise.bin"
# the example plugin, built to WebAssembly (ready-made plugin.wasm, as on a production server)
CARGO_TARGET_DIR=target/plugin-build cargo build -q --release --target wasm32-unknown-unknown     --manifest-path plugins-exemplo/MenuExemplo/Cargo.toml
mkdir -p "$RUN/plugins/MenuExemplo"
cp plugins-exemplo/MenuExemplo/plugin.yml "$RUN/plugins/MenuExemplo/"
cp target/plugin-build/wasm32-unknown-unknown/release/menu_exemplo.wasm "$RUN/plugins/MenuExemplo/plugin.wasm"
if [ -d worlds ]; then tar --exclude=LOCK -cf - worlds | tar -C "$RUN" -xf -; fi
printf "BotOp
BotCraft
BotMiner
BotFood
BotEffect
BotFurnace
BotWater
BotXp
BotLava
BotFarm
BotArcher
BotTarget
BotSleep
BotPotion
BotNoclip
BotGuard
BotFoe
BotBrew
BotMusic
BotEnchant
BotCauldron
BotMenu
" > "$RUN/ops.txt"
cat > "$RUN/server.properties" <<EOF
motd=CeresMC Test
server-port=$PORT
gamemode=creative
level-name=world
allow-unverified-ops=on
xbox-auth=off
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
python -I tools/containers_test.py || status=1
python -I tools/crafting_test.py || status=1
python -I tools/mining_test.py || status=1
python -I tools/food_test.py || status=1
python -I tools/effects_test.py || status=1
python -I tools/furnace_test.py || status=1
python -I tools/twopart_test.py || status=1
python -I tools/doublechest_test.py || status=1
python -I tools/falling_test.py || status=1
python -I tools/liquids_test.py || status=1
python -I tools/xp_test.py || status=1
python -I tools/environment_test.py || status=1
python -I tools/farming_test.py || status=1
python -I tools/projectiles_test.py || status=1
python -I tools/sleep_test.py || status=1
python -I tools/hopper_test.py || status=1
python -I tools/potions_test.py || status=1
python -I tools/noclip_test.py || status=1
python -I tools/defense_test.py || status=1
python -I tools/brewing_test.py || status=1
python -I tools/music_test.py || status=1
python -I tools/enchanting_test.py || status=1
python -I tools/cauldron_test.py || status=1
python -I tools/gameplay_test.py || status=1
python -I tools/packs_test.py || status=1
python -I tools/plugins_test.py || status=1

stop_server
(cd "$RUN" && exec "$EXE" > server2.out 2>&1) &
SERVER=$!
for _ in $(seq 1 100); do grep -q "Done" "$RUN/server2.out" 2>/dev/null && break; sleep 0.1; done
python -I tools/blocks_test.py check || status=1
python -I tools/containers_test.py check || status=1

# third run: Xbox Live verification on, fake logins must be refused
stop_server
sed -i 's/^xbox-auth=off$/xbox-auth=on/' "$RUN/server.properties"
(cd "$RUN" && exec "$EXE" > server3.out 2>&1) &
SERVER=$!
for _ in $(seq 1 100); do grep -q "Done" "$RUN/server3.out" 2>/dev/null && break; sleep 0.1; done
# give the server time to download the keys so the signature check itself is tested
for _ in $(seq 1 150); do grep -q "Xbox Live keys loaded" "$RUN/server3.out" 2>/dev/null && break; sleep 0.1; done
python -I tools/auth_test.py || status=1
python -I tools/nethernet_identity_test.py "$RUN/server3.out" || status=1

if grep -E "bad packet|panicked" "$RUN"/server*.out; then status=1; fi
[ $status -eq 0 ] && echo "ALL TESTS PASSED" || echo "SOME TESTS FAILED"
exit $status
