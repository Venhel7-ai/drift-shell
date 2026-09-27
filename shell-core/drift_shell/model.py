from __future__ import annotations

import copy
import json
import math
import os
import tempfile
import uuid
from dataclasses import dataclass, asdict
from pathlib import Path


class Rejected(ValueError):
    pass


PROFILES = {
    "work": {"right": "Hidden", "dock": "Hidden", "dnd": False},
    "reference": {"right": "Pinned", "dock": "VisibleOverlay", "dnd": False},
    "focus-presentation": {"right": "Hidden", "dock": "Hidden", "dnd": True},
    "minimal": {"right": "Hidden", "dock": "Hidden", "dnd": False},
    "showcase": {"right": "Pinned", "dock": "VisibleOverlay", "dnd": False},
}

# The same metadata drives validation, defaults, search and the settings editor.
SETTINGS = {
    "profile": dict(type="enum", default="work", choices=list(PROFILES), label="Профиль", level=0, group="Внешний вид"),
    "innerGap": dict(type="int", default=12, min=0, max=48, label="Расстояние между окнами", level=1, group="Окна"),
    "outerGap": dict(type="int", default=12, min=0, max=64, label="Внешний отступ", level=1, group="Окна"),
    "masterRatio": dict(type="number", default=.58, min=.15, max=.85, label="Доля главного окна", level=1, group="Окна"),
    "layout": dict(type="enum", default="master", choices=["master", "grid", "columns", "rows", "monocle"], label="Раскладка", level=0, group="Окна"),
    "rightWidth": dict(type="int", default=290, min=240, max=480, label="Ширина правой панели", level=1, group="Панели"),
    "dockReserve": dict(type="bool", default=False, label="Резервировать место дока", level=1, group="Панели"),
    "reducedMotion": dict(type="bool", default=False, label="Уменьшить анимацию", level=0, group="Анимации"),
    "historyLimit": dict(type="int", default=200, min=0, max=1000, label="Размер истории уведомлений", level=1, group="Уведомления"),
    "criticalBypass": dict(type="bool", default=True, label="Критические уведомления при Не беспокоить", level=0, group="Уведомления"),
    "showDate": dict(type="bool", default=False, label="Дата рядом с часами", level=0, group="Панели"),
    "graphPolicy": dict(type="enum", default="ExpandedOnly", choices=["Off", "OnHover", "ExpandedOnly", "Always"], label="Графики показателей", level=1, group="Панели"),
    "starCount": dict(type="int", default=38, min=0, max=100, label="Количество звёзд", level=1, group="Внешний вид"),
}

SETTINGS['profile']['choiceLabels'] = ['Работа', 'Референс', 'Презентация', 'Минимальный', 'Демонстрация']
SETTINGS['layout']['choiceLabels'] = ['Главное и стек', 'Сетка', 'Столбцы', 'Строки', 'Одно окно']
SETTINGS['graphPolicy']['choiceLabels'] = ['Выключены', 'При наведении', 'После раскрытия', 'Всегда']


def defaults():
    return {"schemaVersion": 1, **{k: v["default"] for k, v in SETTINGS.items()}}


def validate_config(candidate):
    if not isinstance(candidate, dict) or candidate.get("schemaVersion") != 1:
        raise Rejected("CFG_SCHEMA_INVALID")
    result = copy.deepcopy(candidate)
    for key, meta in SETTINGS.items():
        value = result.setdefault(key, meta["default"])
        kind = meta["type"]
        valid = ((kind == "bool" and type(value) is bool)
                 or (kind == "int" and type(value) is int)
                 or (kind == "number" and type(value) in (int, float) and math.isfinite(value))
                 or (kind == "enum" and isinstance(value, str) and value in meta["choices"]))
        if not valid or ("min" in meta and not meta["min"] <= value <= meta["max"]):
            raise Rejected("CFG_SEMANTIC_INVALID: " + key)
    return result


def atomic_write(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, name = tempfile.mkstemp(prefix=".pending-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, allow_nan=False, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(name):
            os.unlink(name)


@dataclass(frozen=True)
class Rect:
    x: int
    y: int
    w: int
    h: int


def safe_area(width, height, config, right="Hidden", dock="Hidden"):
    margin = config["outerGap"]
    top = 44 + margin
    reserve_right = config["rightWidth"] + margin if right == "Pinned" else 0
    reserve_bottom = 70 if dock != "Hidden" and config["dockReserve"] else 0
    # Tiny outputs degrade to overlay rather than produce negative cells.
    if width - margin * 2 - reserve_right < 320:
        reserve_right = 0
    if height - top - margin - reserve_bottom < 220:
        reserve_bottom = 0
    return Rect(margin, top, max(1, width - margin * 2 - reserve_right),
                max(1, height - top - margin - reserve_bottom))


def strips(rect, count, gap, vertical=False):
    if not count:
        return []
    extent = rect.h if vertical else rect.w
    space = max(count, extent - gap * (count - 1))
    result = []
    for i in range(count):
        start, end = space * i // count, space * (i + 1) // count
        result.append(Rect(rect.x, rect.y + start + gap * i, rect.w, end - start)
                      if vertical else Rect(rect.x + start + gap * i, rect.y, end - start, rect.h))
    return result


def layout(rect, count, kind="master", gap=12, ratio=.58):
    if count <= 0:
        return []
    if count == 1 or kind == "monocle":
        return [rect] * count
    if kind in ("columns", "rows"):
        cells = strips(rect, count, gap, kind == "rows")
    elif kind == "grid":
        cols = math.ceil(math.sqrt(count))
        rows = math.ceil(count / cols)
        cells = []
        for row in strips(rect, rows, gap, True):
            cells.extend(strips(row, min(cols, count - len(cells)), gap))
    else:
        master_w = round((rect.w - gap) * (.5 if count == 2 else ratio))
        left = Rect(rect.x, rect.y, master_w, rect.h)
        right = Rect(rect.x + master_w + gap, rect.y, rect.w - master_w - gap, rect.h)
        cells = [left] + strips(right, count - 1, gap, True)
    if any(c.w < 320 or c.h < 220 for c in cells):
        return [rect] * count
    return cells


def snap_zone(zones, camera, zoom, viewport, incumbent=None):
    threshold = .35 * math.hypot(*viewport)
    candidates = []
    for zone in zones:
        delta = abs(zone["zoom"] / zoom - 1)
        distance = math.dist(zone["center"], camera) * zoom
        tolerance = 1.1 if zone["id"] == incumbent else 1.0
        if distance < threshold * tolerance and delta <= .12 * tolerance:
            candidates.append((distance / threshold + delta, zone))
    return min(candidates, key=lambda item: item[0])[1] if candidates else None


def visible_slots(active, occupied=(), maximum=5):
    priority = [active, active - 1, active + 1, active - 2, active + 2, *sorted(occupied), *range(1, 10)]
    chosen = []
    for slot in priority:
        if 1 <= slot <= 9 and slot not in chosen:
            chosen.append(slot)
        if len(chosen) == maximum:
            break
    return sorted(chosen)


class Model:
    def __init__(self, config=None):
        self.config = validate_config(config or defaults())
        self.zones = [dict(id=str(uuid.uuid4()), name=f"Область {i}", slot=i,
                           center=[(i - 1) * 10000, 0], zoom=1.0, windows=[])
                      for i in range(1, 10)]
        self.outputs = {}
        self.window_zones = {}
        self.tasks = []
        self.servers = []
        self.focus_snapshot = None
        self.panels = copy.deepcopy(PROFILES[self.config["profile"]])
        self.panels.update(search=False, settings=False, notifications=False)

    def profile(self, name):
        candidate = validate_config({**self.config, "profile": name})
        self.config = candidate
        self.panels.update(PROFILES[name])
        self.focus_snapshot = None

    def toggle_focus(self):
        if self.focus_snapshot is None:
            self.focus_snapshot = copy.deepcopy(self.panels)
            self.panels.update(right="Hidden", dock="Hidden", dnd=True)
        else:
            self.panels = self.focus_snapshot
            self.focus_snapshot = None

    def zone(self, zone_id):
        return next((z for z in self.zones if z["id"] == zone_id), None)

    def dump(self):
        return dict(schemaVersion=1, zones=self.zones, tasks=self.tasks, servers=self.servers,
                    panels=self.focus_snapshot or self.panels, outputs=self.outputs)

    def restore(self, value):
        if value.get("schemaVersion") != 1:
            raise Rejected("CFG_MIGRATION_FAILED")
        zones = value.get("zones")
        if not isinstance(zones, list) or not 1 <= len(zones) <= 1000:
            raise Rejected("CFG_SCHEMA_INVALID")
        ids, slots = set(), set()
        for z in zones:
            if not isinstance(z, dict) or not isinstance(z.get("id"), str) or z["id"] in ids:
                raise Rejected("CFG_SCHEMA_INVALID")
            ids.add(z["id"])
            if z.get("slot") is not None:
                if type(z["slot"]) is not int or not 1 <= z["slot"] <= 9 or z["slot"] in slots:
                    raise Rejected("CFG_SCHEMA_INVALID")
                slots.add(z["slot"])
            if not isinstance(z.get("center"), list) or len(z["center"]) != 2 or any(
                type(v) not in (float, int) or not math.isfinite(v) or abs(v) > 1e8 for v in z["center"]
            ) or type(z.get("zoom")) not in (int, float) or not .01 <= z["zoom"] <= 1:
                raise Rejected("CFG_SCHEMA_INVALID")
        tasks = value.get("tasks", [])
        if not isinstance(tasks, list) or len(tasks) > 1000 or any(
            not isinstance(t, dict) or not isinstance(t.get("title"), str) or
            len(t["title"]) > 500 or type(t.get("done")) is not bool or not isinstance(t.get("id"), str)
            for t in tasks
        ):
            raise Rejected("CFG_SCHEMA_INVALID")
        servers = value.get("servers", [])
        if not isinstance(servers, list) or len(servers) > 100 or any(
            not isinstance(s, dict) or not isinstance(s.get("id"), str) or
            not isinstance(s.get("name"), str) or not isinstance(s.get("host"), str) or
            not 1 <= len(s["host"]) <= 253 or type(s.get("port")) is not int or not 1 <= s["port"] <= 65535
            for s in servers
        ):
            raise Rejected("CFG_SCHEMA_INVALID")
        self.zones = copy.deepcopy(zones)
        for z in self.zones:
            z["windows"] = []  # Compositor handles are valid only within a session.
        self.tasks = copy.deepcopy(tasks)
        self.servers = copy.deepcopy(servers)
        outputs = value.get('outputs', {})
        if isinstance(outputs, dict):
            self.outputs = {name: {'zone': o['zone'], 'anchored': o.get('anchored', True) is True}
                            for name, o in outputs.items() if isinstance(name, str) and
                            isinstance(o, dict) and o.get('zone') in ids}
        panels = value.get("panels", {})
        if panels.get("right") in ("Hidden", "Peek", "Pinned"):
            self.panels["right"] = panels["right"]
        if panels.get("dock") in ("Hidden", "VisibleOverlay"):
            self.panels["dock"] = panels["dock"]
        self.panels["dnd"] = panels.get("dnd") is True
