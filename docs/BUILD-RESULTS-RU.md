# Фактические проверки — 27 сентября 2026

Это отчёт о промежуточной реализации 0.1.0-dev, не акт приёмки полного ТЗ.

## Успешно

- Python unittest: 34 теста, 33 прошли, 1 пропущен. См. test-results.txt.
- Python compileall для shell-core, preview и tests.
- Загрузка общих QML-компонентов через PySide6 6.8.3; offscreen/software PNG
  Work, Reference, pinned и settings сохранены в preview/.
- Rust cargo check --locked: успешно.
- Rust cargo build --locked: собран исполняемый файл в dev-профиле.
- cargo test --locked --no-run: все Rust test targets скомпилированы;
  выполнение Rust-тестов и native integration этим не подтверждается.
- Собранный driftwm --config packaging/arch/driftwm.toml --check-config:
  Config OK, код возврата 0.
- cargo fmt --check и git diff --check: успешно.
- bash -n для установщика, PKGBUILD и скриптов запуска: успешно.

## Среда и ограничения

Проверки выполнялись в Ubuntu 24.04 x86_64, Rust 1.98.1, а не в Arch Linux.
Для сборки системные development-пакеты извлечены в отдельный sysroot;
системные каталоги не изменялись. Старый target от предыдущей среды оказался
несовместим; успешная сборка выполнена в новом отдельном target.

Среда запрещает создание Unix-сокетов (PermissionError / EPERM).
Поэтому тест реального IPC явно пропущен. Нет работающего Wayland-сеанса,
DRM/logind/PAM/greetd окружения: безопасность session-lock, реальное размещение
окон, hotplug и multi-monitor не проверены. Уведомления проверены без D-Bus.

makepkg, установка pacman, release-сборка Arch, Quickshell runtime и Windows
не проверены. Успешная компиляция не доказывает интеграционную готовность.
Бинарный файл Ubuntu не включён в архив: установщик собирает исходники на Arch.

Полный перечень недостающей функциональности находится в ../STATUS-RU.md.
