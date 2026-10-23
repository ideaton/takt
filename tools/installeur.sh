#!/bin/bash
# Construit l'installateur Windows de Takt Studio (Git Bash sous Windows, ou CI GitHub).
#
#   bash tools/installeur.sh
#
# Sortie : target/installeur/TaktStudio-<version>-setup.exe
# La version vient de Cargo.toml (section [workspace.package]).
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

# makensis : celui du PATH (CI), sinon celui installé avec Tauri
MAKENSIS=$(command -v makensis || true)
if [ -z "$MAKENSIS" ]; then
  for c in "$LOCALAPPDATA/tauri/NSIS/makensis.exe" "/c/Program Files (x86)/NSIS/makensis.exe" "/c/Program Files/NSIS/makensis.exe"; do
    if [ -x "$c" ]; then MAKENSIS="$c"; break; fi
  done
fi
if [ -z "$MAKENSIS" ]; then
  echo "makensis introuvable : installez NSIS (https://nsis.sourceforge.io)" >&2
  exit 1
fi

echo "== Compilation $VERSION"
cargo build --release -p takt-ide -p takt-compiler

STAGE=target/installeur/fichiers
rm -rf "$STAGE" && mkdir -p "$STAGE/licences" "$STAGE/exemples"
cp target/release/takt-studio.exe target/release/taktc.exe "$STAGE/"
cp LICENSE "$STAGE/LICENSE.txt"
cp crates/takt-ide/assets/fonts/Inter-LICENSE.txt crates/takt-ide/assets/fonts/lucide-LICENSE.txt "$STAGE/licences/"
cp examples/*.takt "$STAGE/exemples/"
cat > "$STAGE/LISEZMOI.txt" <<EOF
Takt Studio $VERSION
Atelier de programmation d'automates façon TIA Portal, pour RP2350.
Langages : CONT (ladder), SCL, GRAPH, BASIC. Temps de cycle garanti.

Projet d'exemple : Documents\\Takt\\convoyeur.takt
Compilateur en ligne de commande : taktc.exe projet.takt
Sources et documentation : https://github.com/ideaton/takt
Licence : GPL-3.0 (LICENSE.txt). Police Inter (OFL) et icônes Lucide (ISC) : dossier licences.
EOF

SORTIE="target/installeur/TaktStudio-$VERSION-setup.exe"
win() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else echo "$1"; fi; }
echo "== Installateur"
(cd installer && "$MAKENSIS" -V2 "-DVERSION=$VERSION" "-DSRC=$(win "$PWD/../$STAGE")" "-DSORTIE=$(win "$PWD/../$SORTIE")" takt-studio.nsi)
echo "Installateur : $SORTIE ($(du -h "$SORTIE" | cut -f1))"

# Empreinte publiée à côté de l'installateur : la mise à jour intégrée la vérifie
(cd "$(dirname "$SORTIE")" && sha256sum "$(basename "$SORTIE")" > "$(basename "$SORTIE").sha256")
echo "Empreinte : $SORTIE.sha256"
