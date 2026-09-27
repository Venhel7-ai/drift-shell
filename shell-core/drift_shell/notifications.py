import asyncio
import re
import time
from dbus_next.aio import MessageBus
from dbus_next.constants import NameFlag, RequestNameReply
from dbus_next.service import ServiceInterface, method, signal


class Notifications(ServiceInterface):
    def __init__(self, core):
        super().__init__('org.freedesktop.Notifications')
        self.core = core
        self.counter = 0
        self.live = {}
        self.timers = {}

    @method()
    def GetCapabilities(self) -> 'as':
        return ['actions', 'body', 'persistence']

    @method()
    def GetServerInformation(self) -> 'ssss':
        return ['Drift Shell', 'Drift Shell', '0.1.0', '1.2']

    @method()
    def Notify(self, app_name: 's', replaces_id: 'u', app_icon: 's', summary: 's',
               body: 's', actions: 'as', hints: 'a{sv}', expire_timeout: 'i') -> 'u':
        if replaces_id and replaces_id in self.live:
            ident = replaces_id
        else:
            self.counter = self.counter % 0xfffffffe + 1
            ident = self.counter
        urgency = hints.get('urgency')
        urgency = urgency.value if urgency else 1
        entry = dict(id=ident, app=app_name[:200], title=summary[:500],
                     body=re.sub('<[^>]*>', '', body)[:3000], time=time.time(),
                     urgency=urgency, actions=[dict(id=actions[i][:100], title=actions[i+1][:200])
                                              for i in range(0, min(len(actions)-1, 20), 2)],
                     popup=not self.core.model.panels['dnd'] or
                     (urgency == 2 and self.core.model.config['criticalBypass']))
        self.live[ident] = entry
        history = [n for n in self.core.notifications if n['id'] != ident]
        limit = self.core.model.config['historyLimit']
        self.core.notifications = (history + [entry])[-limit:] if limit else []
        # The daemon never writes bodies or replies to disk or logs.
        if ident in self.timers:
            self.timers.pop(ident).cancel()
        if expire_timeout != 0:
            seconds = 6 if expire_timeout < 0 else max(.1, min(3600, expire_timeout/1000))
            self.timers[ident] = asyncio.get_running_loop().call_later(seconds, self.expire, ident)
        while len(self.live) > 1000:
            self.close(next(iter(self.live)), 1)
        self.core.changed_state()
        return ident

    @method()
    def CloseNotification(self, id: 'u'):
        self.close(id, 3)

    @signal()
    def NotificationClosed(self, id, reason) -> 'uu':
        return [id, reason]

    @signal()
    def ActionInvoked(self, id, action_key) -> 'us':
        return [id, action_key]

    def close(self, ident, reason):
        if ident not in self.live:
            return
        entry = self.live.pop(ident)
        entry['popup'] = False
        entry['actions'] = []
        timer = self.timers.pop(ident, None)
        if timer:
            timer.cancel()
        self.NotificationClosed(ident, reason)
        self.core.changed_state()

    def expire(self, ident):
        self.close(ident, 1)

    def invoke(self, ident, action):
        entry = self.live.get(ident)
        if entry and any(a['id'] == action for a in entry['actions']):
            self.ActionInvoked(ident, action)
            self.close(ident, 2)


async def start_notifications(core):
    bus = await MessageBus().connect()
    reply = await bus.request_name('org.freedesktop.Notifications', NameFlag.DO_NOT_QUEUE)
    if reply != RequestNameReply.PRIMARY_OWNER:
        bus.disconnect()
        raise RuntimeError('another notification daemon owns the bus name')
    backend = Notifications(core)
    backend.bus = bus
    bus.export('/org/freedesktop/Notifications', backend)
    return backend
