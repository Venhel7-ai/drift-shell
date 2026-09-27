# Drift Shell 0.1.0-dev

**Это неполная экспериментальная реализация четырёх ТЗ V3, а не готовый релиз.**
Полное соответствие ТЗ, установка на реальной Arch Linux и безопасность полного
сеанса не подтверждены. Конкретные выполненные и отсутствующие части перечислены
в `STATUS-RU.md`. Исходные ТЗ сохранены в `docs/`.

## Что находится в архиве

- Изменённые исходники driftwm 0.19.0; upstream commit
  `352333a8fa1b22171492d4b71a54102045c9a19d`.
- Отдельное Python-ядро состояния и JSON/Unix IPC.
- QML-компоненты интерфейса и Quickshell-адаптер Wayland.
- Исходники отдельных session-lock/PAM и greetd-интерфейсов.
- PKGBUILD, скрипты запуска и systemd user units.
- Переносимый Qt-предпросмотр и тесты; тестовые окна не являются приложениями.

## Установка и запуск на Arch

Склонируйте репозиторий и запустите установщик **от обычного пользователя**:

```bash
git clone https://github.com/Venhel7-ai/drift-shell.git
cd drift-shell
bash install.sh --start
```

Если проект скачан ZIP-архивом, распакуйте его, откройте каталог `drift-shell`
в терминале и выполните:

```bash
bash install.sh --start
```

Скрипт устанавливает зависимости через pacman, собирает fork с Cargo.lock,
запускает проверки упаковки, устанавливает пакет и запускает отдельный сеанс.
Потребуются интернет, подтверждения sudo/pacman и время на компиляцию.
Успешная работа этой команды на реальной Arch в текущей среде не проверена.

Из TTY используется udev/DRM. Из существующего графического сеанса запускается
вложенный winit-сеанс. Нужен рабочий systemd user manager и XDG_RUNTIME_DIR.
Первый запуск лучше выполнять вложенно: это позволяет проверить интерфейс,
не делая его основным рабочим окружением.

Обычный последующий запуск:

```bash
drift-shell-session
```

Также пакет добавляет пункт `Drift Shell (экспериментальная)` в список Wayland-сеансов.
Установщик не включает greetd и не заменяет конфигурацию действующего display manager.
Существующий пакет driftwm сохраняется: fork устанавливается как `driftwm-shell`.

## Клавиши

| Клавиши | Действие |
|---|---|
| Super+Space | Поиск приложений, окон, областей и доступных настроек |
| Super+1…9 | Переключение области |
| Super+Shift+Space | Закрепление / свободная камера |
| Super+R | Правая панель |
| Super+Shift+R | Закрепить / открепить панель |
| Super+D | Док |
| Super+F | Focus с восстановлением прежнего состояния |
| Super+N | Уведомления |
| Super+, | Настройки |
| Super+L | Экспериментальный session-lock |
| Super+Enter | Терминал |
| Super+Shift+Q | Закрыть окно |
| Super+Ctrl+Shift+R | Вернуть профиль Work и открыть настройки |
| Super+Ctrl+Shift+Q | Завершить сеанс |

Блокировку необходимо проверить в отдельном тестовом сеансе. Её аварийное
завершение должно оставлять экран закрытым согласно ext-session-lock-v1;
проверки на реальном композиторе ещё не выполнены.

## Настройки и данные

- `~/.config/drift-shell/config.json` — параметры ядра.
- `~/.config/drift-shell/driftwm.toml` — конфигурация композитора и клавиш.
- `~/.local/state/drift-shell/state.json` — области, задачи и серверы.
- `$XDG_RUNTIME_DIR/drift-shell/core.sock` — локальный сокет текущего пользователя.

XDG_CONFIG_HOME и XDG_STATE_HOME учитываются. При обновлении существующая
конфигурация композитора не перезаписывается. GUI редактирует только параметры,
перечисленные в `schemas/config.schema.json`; это не все параметры исходных ТЗ.
Пароли и содержимое уведомлений не записываются в эти файлы.

Диагностика:

```bash
journalctl --user -u shell-core.service -u shell-ui.service -b
drift-shell-ctl state
```

`state` содержит заголовки окон, задачи и текущие уведомления: не публикуйте его
целиком, если эти данные конфиденциальны. Для безопасного восстановления ядра:

```bash
systemctl --user stop shell-core.service
drift-shell-ctl daemon --safe
```

## Предпросмотр без композитора

Нужны Python 3.11+ и PySide6. На Arch пакет называется `pyside6`.

```bash
python preview/preview.py
python preview/preview.py --profile reference --scene pinned
python preview/preview.py --scene settings --width 1920 --height 1080
```

Windows может использовать те же QML-компоненты через Python/PySide6;
готовый Windows executable не включён и запуск на Windows не проверялся.

PNG предпросмотра содержатся в `docs/preview/`. Это **не golden-приёмка**:
исходные PNG-референсы не приложены, версии шрифтов не закреплены.

## Проверки для разработчика

```bash
PYTHONPATH=shell-core:vendor python -m unittest discover -s tests -v
cargo check --locked --manifest-path driftwm-fork/Cargo.toml
cargo test --locked --manifest-path driftwm-fork/Cargo.toml
cargo fmt --check --manifest-path driftwm-fork/Cargo.toml
```

Тесты ядра используют подставной compositor adapter. Один тест проверяет настоящий
Unix-сокет и явно пропускается в среде, где его создание запрещено.
Тесты уведомлений проверяют обработчик без сеансовой D-Bus шины.

## Удаление

Завершите экспериментальный сеанс и выполните `sudo pacman -R drift-shell`.
Пользовательские настройки и состояние остаются; автоматического удаления данных нет.

## Лицензии

Fork и новый код распространяются по GPL-3.0-or-later. Исходный LICENSE driftwm
сохранён. В `vendor/` включена неизменённая библиотека dbus-next 0.2.3 (MIT),
её лицензия находится в `vendor/dbus_next-0.2.3.dist-info/LICENSE`.
