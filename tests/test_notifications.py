import unittest
from types import SimpleNamespace
from dbus_next import Variant
from drift_shell.model import Model
from drift_shell.notifications import Notifications

class NotificationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.core=SimpleNamespace(model=Model(),notifications=[],changed_state=lambda:None)
        self.server=Notifications(self.core)
    def notify(self,replace=0,hints=None,timeout=0,actions=None):
        return Notifications.Notify.__wrapped__(self.server,'test',replace,'','title','<b>body</b>',actions or [],hints or {},timeout)
    async def test_replacement_preserves_id_and_single_history_item(self):
        a=self.notify();b=self.notify(replace=a);self.assertEqual(a,b);self.assertEqual(len(self.core.notifications),1)
    async def test_markup_is_stripped(self):
        self.notify();self.assertEqual(self.core.notifications[0]['body'],'body')
    async def test_dnd_keeps_history_hides_popup(self):
        self.core.model.panels['dnd']=True;self.notify();self.assertFalse(self.core.notifications[0]['popup'])
    async def test_critical_bypasses_dnd(self):
        self.core.model.panels['dnd']=True;self.notify(hints={'urgency':Variant('y',2)});self.assertTrue(self.core.notifications[0]['popup'])
    async def test_history_bounded(self):
        self.core.model.config['historyLimit']=3
        for i in range(10):self.notify()
        self.assertEqual(len(self.core.notifications),3)
    async def test_expiry_invalidates_actions(self):
        ident=self.notify(actions=['open','Открыть']);self.server.expire(ident)
        self.assertNotIn(ident,self.server.live);self.assertFalse(self.core.notifications[0]['popup']);self.assertEqual(self.core.notifications[0]['actions'],[])
    async def test_disabled_history_still_has_live_popup(self):
        self.core.model.config['historyLimit']=0;ident=self.notify();self.assertEqual(self.core.notifications,[]);self.assertTrue(self.server.live[ident]['popup'])

if __name__=='__main__':unittest.main()
