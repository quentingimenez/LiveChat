#!/usr/bin/env bash
# Installe les bibliothèques système Linux nécessaires à la compilation du
# workspace (apps/lc-overlay, Tauri 2) sur un runner GitHub ubuntu-latest.
#
# Constat mesuré (CI du 2026-11-24) : l'image du runner pointe sa source Ubuntu
# sur `mirror+file:/etc/apt/apt-mirrors.txt`, et la liste de miroirs donne la
# priorité 1 au miroir régional de la région qui a construit l'image
# (`us-east-2.ec2.archive.ubuntu.com` sur les images AWS). Ce miroir descend
# régulièrement à ~50 kB/s : les 63,8 Mo de paquets ont mis 19 min 57 s au lieu
# de 15 s sur un autre runner, ajoutant 20 min au job `test (ubuntu-latest)`.
# Les runners sont hébergés sur Azure et `azure.archive.ubuntu.com` a servi les
# mêmes paquets à 4,1 Mo/s ; les téléchargements non-apt du même job (Chromium
# Playwright, nextest) n'étaient pas ralentis — c'est le miroir, pas le réseau.
#
# Le script épingle donc la source sur le miroir Azure. Si `apt-get update`
# échoue, il restaure la configuration d'origine et réessaie avec le miroir
# par défaut : une panne du miroir Azure ne doit pas casser le gate, seulement
# le ralentir comme avant.
set -euo pipefail

readonly packages=(
  build-essential
  curl
  wget
  file
  libwebkit2gtk-4.1-dev
  libgtk-3-dev
  libxdo-dev
  libayatana-appindicator3-dev
  librsvg2-dev
  libssl-dev
  pkg-config
  patchelf
)

# Fichiers pouvant porter `mirror+file:/etc/apt/apt-mirrors.txt` : format deb822
# des images récentes, et format une-ligne historique.
readonly sources_files=(
  /etc/apt/sources.list.d/ubuntu.sources
  /etc/apt/sources.list
)
readonly backup_dir=/tmp/lc-apt-sources-backup

use_azure_mirror() {
  local file
  local rewritten=0
  sudo rm -rf "$backup_dir"
  sudo mkdir -p "$backup_dir"
  for file in "${sources_files[@]}"; do
    [[ -f "$file" ]] || continue
    grep -qF 'mirror+file:/etc/apt/apt-mirrors.txt' "$file" || continue
    sudo cp "$file" "$backup_dir/$(basename "$file")"
    sudo sed -i 's|mirror+file:/etc/apt/apt-mirrors.txt|http://azure.archive.ubuntu.com/ubuntu|g' "$file"
    rewritten=$((rewritten + 1))
  done
  [[ "$rewritten" -gt 0 ]]
}

restore_sources() {
  local file
  for file in "${sources_files[@]}"; do
    [[ -f "$backup_dir/$(basename "$file")" ]] || continue
    sudo cp "$backup_dir/$(basename "$file")" "$file"
  done
  sudo rm -rf "$backup_dir"
}

if use_azure_mirror; then
  if ! sudo apt-get update; then
    echo "::warning::miroir Azure injoignable, retour au miroir par défaut du runner"
    restore_sources
    sudo apt-get update
  fi
  sudo rm -rf "$backup_dir"
else
  # Image sans liste de miroirs : rien à épingler, comportement d'origine.
  sudo apt-get update
fi

DEBIAN_FRONTEND=noninteractive sudo apt-get install -y --no-install-recommends "${packages[@]}"
