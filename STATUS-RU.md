# Фактическое состояние — 0.1.0-dev

Запрошенная пользователем полная реализация ТЗ **НЕ завершена**.
Этот архив сохраняет выполненную реализацию и позволяет продолжить работу.
Наличие файла/класса не считается доказательством готовности соответствующего
раздела ТЗ. Рабочий сеанс Arch Linux здесь не запускался.

## Реализовано в исходниках и проверено на уровне логики

- Пять вариантов раскладки: master/stack, grid, columns, rows, monocle.
- Переход к monocle вместо слишком маленьких ячеек.
- Единый Safe Area; Peek и overlay-док не отнимают площадь.
- Переключение пяти профилей без изменения списков окон областей.
- Focus snapshot и точное восстановление прежних панелей/DND.
- Девять быстрых областей, поиск ближайшей области по screen-space и zoom.
- Атомарная запись JSON: временный файл, fsync, rename; валидация перед commit.
- Ограниченный поиск .desktop-приложений, окон, областей и доступных параметров.
- Локальные задачи; TCP-проверки явно добавленных серверов, offline после трёх ошибок.
- CPU/network каждую секунду, memory каждые 2 секунды, disk каждые 10 секунд.
- Ограниченный буфер метрик, независимый от отображения графика.
- Обработчик Notifications: замена, действия, timeout, DND, ограниченная история,
  удаление разметки, отсутствие записи содержимого уведомлений на диск.
- Проверка версии команд и UID Unix-клиента, права сокета 0600.

## Написано, но интеграционная готовность не подтверждена

- Fork: ShellCamera, блокировка ряда путей pan/zoom и ShellLayout с предварительной
  проверкой всей пачки окон. Камера устанавливается сразу; анимации 250 мс нет.
- Пересчёт окон при pin/unpin, reconnect, появлении/исчезновении окна.
- Обработка transient через отдельный признак — такие окна не включаются в tiling.
- Перенос окон отключённого output на оставшуюся область: алгоритм есть,
  реальный hotplug/DPI/rotation не проверен.
- Quickshell surfaces, маски ввода, видимость панелей при fullscreen.
- QML session-lock использует ext-session-lock-v1 и Quickshell PAM helper.
- QML greeter использует greetd API. Он не устанавливается как активный display manager.
- Arch PKGBUILD и однокомандный установщик: не проверены в настоящей Arch Linux.

## Отсутствует или существенно не соответствует полной версии ТЗ

1. Layout tree Split/Stack/Leaf/Placeholder, per-zone ratios, drag/reorder и
   интерактивное изменение границ tiled-окон; правила app admission по priority/specificity.
2. Полная модель WindowState, per-window tiled/floating/pinned modes и история
   tile-path. FreeCanvas окна сейчас явно не включаются в область автоматически.
3. Точная V3 state machine камеры, прерываемая анимация камеры/окон и события
   AnimationCompleted, granular compositor events, revision replay protection.
4. Панели сохраняют конечные состояния; PreparingPin/Unpinning и координация
   на 55–65% reflow не реализованы. Нет edge/swipe reveal, timeout/click-outside,
   гарантированного возврата keyboard focus и полного dock application grouping.
5. Настройки всех public properties, полная иерархия scope/inheritance, миграции,
   confirm-timer risky changes, import/export, редактирование hotkeys с конфликтами.
6. Design Mode с контейнерами, drag/resize, snap guides и undo на 100 операций.
7. Полный набор интеграций PipeWire, NetworkManager, BlueZ, UPower,
   power-profiles-daemon, MPRIS, tray, logind и OSD.
8. Полный search по файлам/явным корням, закладкам браузера, серверам,
   команды, fuzzy/recency/pinning и контроль времени первых результатов.
9. Zen userChrome/new-tab пакет, совместимость с версиями, backup/rollback,
   extension, native messaging host и реальные quick-reply adapters.
   В zen-integration находятся только сгенерированные CSS color tokens.
10. Plugin/widget manifests, permission model и установка сторонних виджетов.
11. Полный серверный мониторинг с SSH/HTTP/ICMP, secret service и параметрами.
    Здесь доступны только TCP-connect проверки с фиксированным интервалом.
12. Точные анимации lock, PAM multi-factor UX, оформление greeter с перечнем
    пользователей/сессий, power actions и полноценная multi-monitor раскладка.
13. Security acceptance: kill UI/core/locker, PAM errors, hotplug и suspend/resume.
14. Crash-loop safe mode и полный rollback browser/compositor/profile upgrades.
15. Точное оформление Level A/B/C blur/shadows, единый SVG icon pack,
    закреплённые версии шрифтов и материал окон сторонних приложений.
16. Эталонная геометрия REFERENCE/G01..G18, W01..W10, masks/geometry diff harness.
17. Windows executable и реальные Windows/DPI/multi-monitor тесты.
18. Power-aware mode, замеры CPU/RSS/FPS, soak-тесты 1000 панелей/500 областей.
19. Состояния панелей пока общие, а не индивидуальные для каждого output.
    Output identity основана на имени, а не устойчивом hardware fingerprint.
20. Полное восстановление назначения приложений по app_id/role после нового
    compositor-сеанса. Старые session-local window IDs намеренно отбрасываются.

## Как читать результаты

`docs/test-results.txt` — фактический вывод Python unittest.
Qt-предпросмотр проверяет загрузку общих UI-компонентов и геометрию изображений,
но не Quickshell integration, PAM, greetd или реальные окна.
`docs/BUILD-RESULTS-RU.md` описывает результат сборки fork и ограничения среды.

Весь Definition of Done из MASTER SPEC остаётся невыполненным. Этот архив
нельзя переименовывать в FINAL или считать безопасным производственным релизом.
