#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ ${EUID} -eq 0 ]]; then
  echo 'Запускайте от обычного пользователя: makepkg не работает от root.' >&2; exit 1
fi
if [[ ! -f /etc/arch-release ]] || ! command -v pacman >/dev/null; then
  echo 'Установщик предназначен для Arch Linux.' >&2; exit 1
fi
case "${1:-}" in ''|--start) ;; *) echo 'Использование: bash install.sh [--start]' >&2; exit 2 ;; esac
cat <<'MSG'
Drift Shell 0.1.0-dev — неполная экспериментальная реализация.
Полное соответствие ТЗ и проверки реального Arch-сеанса НЕ подтверждены.
Подробности: STATUS-RU.md. Сборка требует интернета, места и Rust >= 1.88.
MSG
sudo pacman -Syu --needed base-devel rust python quickshell ttf-inter ttf-jetbrains-mono
cd -- "$project_dir"
makepkg --syncdeps --install --cleanbuild
systemctl --user daemon-reload
if [[ ${1:-} == --start ]]; then exec /usr/bin/drift-shell-session; fi
printf '%s\n' 'Пакет установлен. Запуск: drift-shell-session (из TTY или вложенно из Wayland).'
