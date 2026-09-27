from __future__ import annotations

import asyncio
import copy
import configparser
import fcntl
import json
import os
import shutil
import socket
import struct
import time
import uuid
from collections import deque
from pathlib import Path

from .model import Model, Rejected, SETTINGS, atomic_write, defaults, layout, safe_area, snap_zone, validate_config, visible_slots

MAX_LINE = 262144


def encode(value):
    return (json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":")) + "\n").encode()


def directories():
    home = Path.home()
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if not runtime:
        raise RuntimeError("XDG_RUNTIME_DIR не задан; запустите оболочку в пользовательском сеансе")
    return (Path(os.environ.get("XDG_CONFIG_HOME", home / ".config")) / "drift-shell",
            Path(os.environ.get("XDG_STATE_HOME", home / ".local/state")) / "drift-shell",
            Path(runtime) / "drift-shell")


class Compositor:
    def __init__(self):
        display = os.environ.get("WAYLAND_DISPLAY", "wayland-0")
        self.path = os.environ.get("DRIFTWM_SOCKET", f"{os.environ.get('XDG_RUNTIME_DIR')}/driftwm/ipc-{display}.sock")

    async def call(self, request):
        reader, writer = await asyncio.wait_for(asyncio.open_unix_connection(self.path, limit=MAX_LINE), 2)
        try:
            writer.write(encode(request))
            await writer.drain()
            value = json.loads(await asyncio.wait_for(reader.readline(), 3))
            if "Err" in value:
                raise Rejected(value["Err"])
            return value["Ok"]
        finally:
            writer.close()
            await writer.wait_closed()

    async def events(self):
        reader, writer = await asyncio.open_unix_connection(self.path, limit=4 * MAX_LINE)
        try:
            writer.write(encode("Subscribe"))
            await writer.drain()
            while line := await reader.readline():
                message = json.loads(line)
                if "State" in message:
                    yield message["State"]
            raise ConnectionError("IPC_DISCONNECTED")
        finally:
            writer.close()
            await writer.wait_closed()


class Core:
    def __init__(self, safe=False):
        self.config_dir, self.state_dir, self.runtime_dir = directories()
        self.config_path = self.config_dir / "config.json"
        self.state_path = self.state_dir / "state.json"
        self.errors = deque(maxlen=40)
        self.model = Model()
        if not safe and self.config_path.exists():
            try:
                self.model = Model(json.loads(self.config_path.read_text()))
            except (OSError, ValueError):
                self.errors.append("CFG_SCHEMA_INVALID: загружен безопасный профиль")
        if not safe and self.state_path.exists():
            try:
                self.model.restore(json.loads(self.state_path.read_text()))
            except (OSError, ValueError, KeyError, TypeError):
                self.model = Model(self.model.config)
                self.errors.append("CFG_SCHEMA_INVALID: состояние не восстановлено")
        self.safe = safe
        self.wm = Compositor()
        self.connected = False
        self.clients = set()
        self.raw = {"outputs": [], "windows": [], "fullscreen": []}
        self.metrics = {}
        self.samples = deque(maxlen=60)
        self.revision = 0
        self.accepted = 0
        self.last_targets = None
        self.apps = []
        self.notifications = []
        self.notification_backend = None
        self.mutex = asyncio.Lock()
        self.event_id = 0
        self.changed = asyncio.Event()
        self.persist_pending = False
        self.fullscreen_ids = set()
        self.server_status = {}
        self.persisted_value = None

    def current_output(self):
        return next((o for o in self.raw["outputs"] if o.get("active")),
                    next(iter(self.raw["outputs"]), None))

    def current_zone(self):
        output = self.current_output()
        if not output:
            return self.model.zones[0]
        record = self.model.outputs.get(output["name"], {})
        return self.model.zone(record.get("zone")) or self.model.zones[0]

    def snapshot(self):
        zone = self.current_zone()
        slots = list(range(1, 10)) if self.model.config["profile"] in ("reference", "showcase") else visible_slots(zone.get("slot") or 1)
        return dict(schemaVersion=1, config=self.model.config, settings=SETTINGS,
                    panels=self.model.panels, focus=self.model.focus_snapshot is not None,
                    zones=self.model.zones, activeZone=zone["id"], slots=slots,
                    outputs=self.model.outputs, compositor=self.raw,
                    connected=self.connected, metrics=self.metrics, samples=list(self.samples),
                    tasks=self.model.tasks, notifications=self.notifications,
                    popups=[n for n in self.notification_backend.live.values() if n['popup']] if self.notification_backend else [],
                    servers=[{**s, **self.server_status.get(s['id'], {'status':'unknown'})} for s in self.model.servers],
                    notificationAvailable=self.notification_backend is not None,
                    acceptedRevision=self.accepted, errors=list(self.errors), safeMode=self.safe)

    def changed_state(self, persist=False):
        self.changed.set()
        self.persist_pending |= persist

    async def publisher(self):
        while True:
            await self.changed.wait()
            await asyncio.sleep(.03)
            self.changed.clear()
            self.event_id += 1
            packet = encode(dict(eventId=self.event_id, timestamp=time.time(), type="State", payload=self.snapshot()))
            for writer in tuple(self.clients):
                try:
                    writer.write(packet)
                    await asyncio.wait_for(writer.drain(), .2)
                except (OSError, asyncio.TimeoutError):
                    self.clients.discard(writer)
                    writer.close()
            if self.persist_pending:
                self.persist_pending = False
                value = copy.deepcopy(self.model.dump())
                if value != self.persisted_value:
                    await asyncio.to_thread(atomic_write, self.state_path, value)
                    self.persisted_value = value

    async def camera(self, output, zone, anchored=True):
        await self.wm.call({"ShellCamera": dict(output=output["name"], center=zone["center"], zoom=zone["zoom"], anchored=anchored)})

    async def reconcile(self, raw):
        previous_names = {o["name"] for o in self.raw["outputs"]}
        self.raw = raw
        names = {o["name"] for o in raw["outputs"]}
        for output in raw["outputs"]:
            name = output["name"]
            if name not in self.model.outputs:
                used = {v["zone"] for k, v in self.model.outputs.items() if k in names}
                zone = next((z for z in self.model.zones if z["id"] not in used), self.model.zones[0])
                self.model.outputs[name] = dict(zone=zone["id"], anchored=True)
            if name not in previous_names:
                record = self.model.outputs[name]
                zone = self.model.zone(record['zone'])
                await self.camera(output, zone, record['anchored'])
            self.model.outputs[name]["size"] = output["size"]
        output = self.current_output()
        if not output:
            self.changed_state()
            return
        fallback = self.current_zone()
        # Move the contents of a disconnected viewport into a reachable zone.
        for name in previous_names - names:
            record = self.model.outputs.get(name)
            lost = self.model.zone(record["zone"]) if record else None
            if lost and lost != fallback:
                for window_id in lost["windows"]:
                    if window_id not in fallback["windows"]:
                        fallback["windows"].append(window_id)
                    self.model.window_zones[window_id] = fallback["id"]
                lost["windows"] = []
        fullscreen = {w["id"] for w in raw.get("fullscreen", [])}
        pinned = {w["id"] for w in raw.get("pinned", [])}
        live = {w["id"] for w in raw["windows"]} | fullscreen | pinned
        for z in self.model.zones:
            z["windows"] = [w for w in z["windows"] if w in live]
        self.model.window_zones = {k: v for k, v in self.model.window_zones.items() if k in live}
        for window in raw["windows"]:
            ident = window["id"]
            if window.get("is_widget") or window.get("transient") or ident in self.model.window_zones:
                continue
            # Free-canvas windows are explicitly unassigned, never inherited.
            zone_id = fallback["id"] if self.model.outputs[output["name"]]["anchored"] else None
            self.model.window_zones[ident] = zone_id
            if zone_id:
                fallback["windows"].append(ident)
        self.fullscreen_ids = fullscreen
        try:
            await self.reflow()
        except Rejected as error:
            message = str(error)
            if not self.errors or self.errors[-1] != message:
                self.errors.append(message)
        self.changed_state(persist=True)

    async def reflow(self):
        targets = []
        config = self.model.config
        available = {w["id"] for w in self.raw["windows"] if not w.get("is_widget")}
        for output in self.raw["outputs"]:
            state = self.model.outputs.get(output["name"])
            if not state or not state["anchored"]:
                continue
            zone = self.model.zone(state["zone"])
            if not zone:
                continue
            windows = [i for i in zone["windows"] if i in available]
            rect = safe_area(*output["size"], config, self.model.panels["right"], self.model.panels["dock"])
            cells = layout(rect, len(windows), config["layout"], config["innerGap"], config["masterRatio"])
            for ident, cell in zip(windows, cells):
                zoom = zone["zoom"]
                targets.append(dict(id=ident,
                    x=round(zone["center"][0] + (cell.x + cell.w / 2 - output["size"][0] / 2) / zoom),
                    y=round(zone["center"][1] - (cell.y + cell.h / 2 - output["size"][1] / 2) / zoom),
                    w=round(cell.w / zoom), h=round(cell.h / zoom)))
        if targets == self.last_targets:
            return
        self.revision += 1
        await self.wm.call({"ShellLayout": dict(revision=self.revision, targets=targets)})
        self.accepted = self.revision
        self.last_targets = targets

    async def watch_compositor(self):
        delay = .25
        while True:
            try:
                # A new full snapshot is mandatory after reconnect.
                raw = (await self.wm.call("State"))["State"]
                async with self.mutex:
                    self.last_targets = None
                    await self.reconcile(raw)
                    self.connected = True
                    self.changed_state()
                delay = .25
                async for raw in self.wm.events():
                    async with self.mutex:
                        await self.reconcile(raw)
            except (OSError, ValueError, KeyError, asyncio.TimeoutError):
                if self.connected:
                    self.errors.append("IPC_DISCONNECTED: соединение с композитором потеряно")
                self.connected = False
                self.changed_state()
                await asyncio.sleep(delay)
                delay = min(5, delay * 2)

    async def command(self, action, payload):
        m = self.model
        if action == "state":
            return self.snapshot()
        if action == "search":
            query = str(payload.get("query", ""))[:200].casefold().strip()
            candidates = [dict(kind="app", id=a["id"], title=a["name"]) for a in self.apps]
            candidates += [dict(kind="zone", id=z["id"], title=z["name"]) for z in m.zones]
            candidates += [dict(kind="window", id=w["id"], title=w.get("title") or w["app_id"]) for w in self.raw["windows"]]
            candidates += [dict(kind="setting", id=k, title=v["label"]) for k, v in SETTINGS.items()]
            candidates = [v for v in candidates if query in v["title"].casefold()]
            candidates.sort(key=lambda v: (not v["title"].casefold().startswith(query), v["title"].casefold()))
            return candidates[:30]
        if action == "launch":
            app = next((a for a in self.apps if a["id"] == payload.get("id")), None)
            if not app:
                raise Rejected("APP_NOT_FOUND")
            await asyncio.create_subprocess_exec("gio", "launch", app["path"], stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL)
            m.panels["search"] = False
        elif action == "window":
            await self.wm.call({"Focus": int(payload["id"])})
            m.panels["search"] = False
        elif action == "zone":
            zone = m.zone(payload.get("id")) if "id" in payload else next((z for z in m.zones if z.get("slot") == payload.get("slot")), None)
            output = self.current_output()
            if not zone or not output:
                raise Rejected("ZONE_NOT_FOUND")
            if any(k != output["name"] and o["zone"] == zone["id"] for k, o in m.outputs.items() if k in {x["name"] for x in self.raw["outputs"]}):
                raise Rejected("ZONE_ON_OTHER_OUTPUT")
            await self.camera(output, zone)
            m.outputs[output["name"]].update(zone=zone["id"], anchored=True)
            m.panels["search"] = False
            await self.reflow()
        elif action == "anchor":
            output = self.current_output()
            if not output:
                raise Rejected("OUTPUT_PROFILE_MISSING")
            record = m.outputs[output["name"]]
            current = dict(center=list(output["camera"]), zoom=output["zoom"])
            if record["anchored"]:
                await self.camera(output, current, False)
                record["anchored"] = False
            else:
                zone = snap_zone(m.zones, current["center"], current["zoom"], output["size"], record["zone"])
                if zone is None:
                    zone = dict(id=str(uuid.uuid4()), name=f"Область {len(m.zones)+1}", slot=None, windows=[], **current)
                    m.zones.append(zone)
                await self.camera(output, zone)
                record.update(zone=zone["id"], anchored=True)
                await self.reflow()
        elif action in ("right", "pin", "dock", "focus", "search-toggle", "settings", "notifications", "dnd"):
            before = copy.deepcopy(m.panels)
            focus_before = copy.deepcopy(m.focus_snapshot)
            if action == "right":
                m.panels["right"] = "Peek" if m.panels["right"] == "Hidden" else "Hidden"
            elif action == "pin":
                m.panels["right"] = "Peek" if m.panels["right"] == "Pinned" else "Pinned"
            elif action == "dock":
                m.panels["dock"] = "VisibleOverlay" if m.panels["dock"] == "Hidden" else "Hidden"
            elif action == "focus":
                m.toggle_focus()
            else:
                key = "search" if action == "search-toggle" else action
                m.panels[key] = not m.panels[key]
            try:
                await self.reflow()
            except Exception:
                m.panels, m.focus_snapshot = before, focus_before
                raise
        elif action == "dismiss":
            for key in ("search", "settings", "notifications"):
                m.panels[key] = False
            if m.panels["right"] == "Peek":
                m.panels["right"] = "Hidden"
        elif action == "configure":
            changes = payload.get("changes")
            if not isinstance(changes, dict) or any(k not in SETTINGS for k in changes):
                raise Rejected("CFG_SCHEMA_INVALID")
            candidate = validate_config({**m.config, **changes})
            old = copy.deepcopy(m.config)
            old_panels, old_focus = copy.deepcopy(m.panels), copy.deepcopy(m.focus_snapshot)
            m.config = candidate
            if "profile" in changes:
                m.profile(candidate["profile"])
            try:
                await self.reflow()
                if self.config_path.exists():
                    await asyncio.to_thread(atomic_write, self.config_dir / "config.previous.json", old)
                await asyncio.to_thread(atomic_write, self.config_path, candidate)
            except Exception:
                m.config, m.panels, m.focus_snapshot = old, old_panels, old_focus
                self.last_targets = None
                await self.reflow()
                raise
        elif action == "task-add":
            title = payload.get("title", "").strip()
            if not title or len(title) > 500 or len(m.tasks) >= 1000:
                raise Rejected("TASK_INVALID")
            m.tasks.append(dict(id=str(uuid.uuid4()), title=title, done=False))
        elif action == "server-add":
            name, host, port = payload.get('name'), payload.get('host'), payload.get('port')
            if not isinstance(name, str) or not 1 <= len(name) <= 80 or not isinstance(host, str) or not 1 <= len(host) <= 253 or type(port) is not int or not 1 <= port <= 65535 or len(m.servers) >= 100:
                raise Rejected('SERVER_INVALID')
            m.servers.append(dict(id=str(uuid.uuid4()), name=name, host=host, port=port))
        elif action == "server-delete":
            m.servers = [s for s in m.servers if s['id'] != payload.get('id')]
        elif action in ("task-toggle", "task-delete"):
            task = next((t for t in m.tasks if t["id"] == payload.get("id")), None)
            if not task:
                raise Rejected("TASK_NOT_FOUND")
            if action == "task-delete":
                m.tasks.remove(task)
            else:
                task["done"] = not task["done"]
        elif action == "notification-clear":
            if self.notification_backend:
                for ident in list(self.notification_backend.live):
                    self.notification_backend.close(ident, 2)
            self.notifications = []
        elif action == "notification-action":
            if not self.notification_backend:
                raise Rejected("WIDGET_BACKEND_UNAVAILABLE")
            self.notification_backend.invoke(int(payload["id"]), str(payload["action"]))
        elif action == "safe-reset":
            m.profile("work")
            m.panels.update(search=False, settings=True, notifications=False)
            await self.reflow()
        else:
            raise Rejected("UNKNOWN_ACTION")
        self.changed_state(persist=True)
        return {"accepted": True}

    async def client(self, reader, writer):
        try:
            # Restrict peer UID even if a parent runtime directory is misconfigured.
            credentials = writer.get_extra_info("socket").getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
            if struct.unpack("3i", credentials)[1] != os.getuid():
                return
            self.clients.add(writer)
            writer.write(encode(dict(type="State", payload=self.snapshot())))
            await writer.drain()
            while line := await reader.readline():
                request = None
                try:
                    request = json.loads(line)
                    if not isinstance(request, dict) or request.get("schemaVersion") != 1 or not isinstance(request.get("payload", {}), dict):
                        raise Rejected("IPC_PROTOCOL_MISMATCH")
                    async with self.mutex:
                        result = await self.command(request["action"], request.get("payload", {}))
                    response = dict(requestId=request.get("requestId"), ok=True, result=result)
                except (ValueError, KeyError, TypeError, OSError, asyncio.TimeoutError) as error:
                    response = dict(requestId=request.get("requestId") if isinstance(request, dict) else None,
                                    ok=False, errorCode=str(error)[:160], errorMessage="Не удалось выполнить действие")
                writer.write(encode(response))
                await asyncio.wait_for(writer.drain(), 2)
        except (OSError, ValueError, asyncio.TimeoutError):
            pass
        finally:
            self.clients.discard(writer)
            writer.close()

    def scan_apps(self):
        paths = [Path(os.environ.get("XDG_DATA_HOME", Path.home()/".local/share"))]
        paths += [Path(p) for p in os.environ.get("XDG_DATA_DIRS", "/usr/local/share:/usr/share").split(":")]
        found = {}
        seen = set()
        for base in paths:
            appdir = base / "applications"
            for path in sorted(appdir.rglob("*.desktop")):
                ident = str(path.relative_to(appdir)).replace("/", "-")
                if ident in seen:
                    continue
                seen.add(ident)
                parser = configparser.ConfigParser(interpolation=None, strict=False)
                try:
                    parser.read(path, encoding="utf-8")
                    section = parser["Desktop Entry"]
                    if section.get("Type") != "Application" or section.getboolean("Hidden", fallback=False) or section.getboolean("NoDisplay", fallback=False):
                        continue
                    executable = section.get("TryExec")
                    if executable and not shutil.which(executable):
                        continue
                    name = section.get("Name[ru]", section.get("Name", ident))
                    found[ident] = dict(id=ident, name=name, path=str(path))
                except (OSError, ValueError, KeyError, configparser.Error):
                    continue
        return list(found.values())

    async def sampler(self):
        previous = None
        previous_net = None
        tick = 0
        cached_memory = 0.0
        cached_disk = 0.0
        while True:
            try:
                def sample():
                    cpu = list(map(int, Path("/proc/stat").read_text().splitlines()[0].split()[1:9]))
                    mem = {line.split(":")[0]: int(line.split()[1]) for line in Path("/proc/meminfo").read_text().splitlines()} if tick % 2 == 0 else None
                    network = [line.replace(":", " ").split() for line in Path("/proc/net/dev").read_text().splitlines()[2:]]
                    down = sum(int(line[1]) for line in network if line[0] != "lo")
                    up = sum(int(line[9]) for line in network if line[0] != "lo")
                    disk = shutil.disk_usage(Path.home()) if tick % 10 == 0 else None
                    return cpu, mem, down, up, disk
                cpu, mem, down, up, disk = await asyncio.to_thread(sample)
                total, idle = sum(cpu), cpu[3] + cpu[4]
                value = 0.0 if previous is None or total == previous[0] else 100 * (1 - (idle - previous[1]) / (total - previous[0]))
                now = time.monotonic()
                seconds = now - previous_net[2] if previous_net else 1
                if mem is not None:
                    cached_memory = round(100 * (1-mem['MemAvailable']/mem['MemTotal']), 1)
                if disk is not None:
                    cached_disk = round(100 * disk.used/disk.total, 1)
                self.metrics = dict(cpu=round(max(0, min(100, value)), 1),
                    memory=cached_memory, disk=cached_disk,
                    download=round(max(0, down-previous_net[0])/seconds) if previous_net else 0,
                    upload=round(max(0, up-previous_net[1])/seconds) if previous_net else 0)
                previous, previous_net = (total, idle), (down, up, now)
                self.samples.append(self.metrics["cpu"])
                self.changed_state()
            except (OSError, ValueError, KeyError):
                pass
            tick += 1
            await asyncio.sleep(1)

    async def monitor_servers(self):
        limit = asyncio.Semaphore(8)
        async def check(server):
            async with limit:
                started = time.monotonic()
                ident = server['id']
                previous = self.server_status.get(ident, {'failures':0, 'status':'unknown'})
                try:
                    reader, writer = await asyncio.wait_for(asyncio.open_connection(server['host'], server['port']), 3)
                    writer.close()
                    await writer.wait_closed()
                    self.server_status[ident] = dict(status='online', failures=0, latency=round((time.monotonic()-started)*1000))
                except (OSError, asyncio.TimeoutError):
                    failures = previous.get('failures', 0) + 1
                    self.server_status[ident] = dict(status='offline' if failures >= 3 else previous['status'], failures=failures, latency=None)
        while True:
            await asyncio.gather(*(check(dict(s)) for s in self.model.servers))
            ids = {s['id'] for s in self.model.servers}
            self.server_status = {k:v for k,v in self.server_status.items() if k in ids}
            self.changed_state()
            await asyncio.sleep(15)

    async def run(self):
        self.runtime_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(self.runtime_dir, 0o700)
        self.lock_file = open(self.runtime_dir / "daemon.lock", "a")
        fcntl.flock(self.lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        path = self.runtime_dir / "core.sock"
        path.unlink(missing_ok=True)
        server = await asyncio.start_unix_server(self.client, path=str(path), limit=MAX_LINE)
        os.chmod(path, 0o600)
        self.apps = await asyncio.to_thread(self.scan_apps)
        try:
            from .notifications import start_notifications
            self.notification_backend = await start_notifications(self)
        except Exception:
            # Optional service must not take down the layout authority on a bus failure.
            self.errors.append("WIDGET_BACKEND_UNAVAILABLE: служба уведомлений недоступна")
        async with server:
            async with asyncio.TaskGroup() as group:
                group.create_task(server.serve_forever())
                group.create_task(self.watch_compositor())
                group.create_task(self.sampler())
                group.create_task(self.monitor_servers())
                group.create_task(self.publisher())
