#!/usr/bin/env bash
# Compila (se o codigo mudou) e inicia o CeresMC Server a partir desta pasta.
cd "$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd)" || exit 1
export PATH="$HOME/.cargo/bin:$PATH"; CARGO=cargo
[ -f server.properties ] || cp server.properties.example server.properties
mkdir -p worlds
echo "Compilando (leva poucos segundos se nada mudou)..."
"$CARGO" build --release || { echo "Falha ao compilar."; exit 1; }
echo; echo "Para desligar salvando tudo: digite stop ou aperte Ctrl+C"; echo
./target/release/ceres
