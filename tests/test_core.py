import asyncio
import copy
import json
import os
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from drift_shell.daemon import Core, encode
from drift_shell.model import Rejected

class FakeCompositor:
    def __init__(self):self.calls=[];self.fail=False
    async def call(self,value):
        if self.fail:raise Rejected('simulated rejection')
        self.calls.append(value)
        return 'Ok'

class CoreTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.directory=Path(self.tmp.name)
        self.env=patch.dict(os.environ,{'XDG_CONFIG_HOME':str(self.directory/'config'),'XDG_STATE_HOME':str(self.directory/'state'),'XDG_RUNTIME_DIR':str(self.directory/'runtime')});self.env.start()
        self.core=Core();self.core.wm=FakeCompositor()
        self.raw={'outputs':[{'name':'TEST-1','active':True,'camera':[0,0],'zoom':1,'size':[1672,941]}],
                  'windows':[{'id':1,'app_id':'test','title':'test','position':[0,0],'size':[600,400]}], 'pinned':[], 'fullscreen':[]}
        await self.core.reconcile(copy.deepcopy(self.raw))
    async def asyncTearDown(self):self.env.stop();self.tmp.cleanup()
    async def test_peek_does_not_submit_new_layout(self):
        count=len(self.core.wm.calls);await self.core.command('right',{});self.assertEqual(count,len(self.core.wm.calls))
    async def test_pin_submits_layout(self):
        old=self.core.accepted;await self.core.command('pin',{});self.assertGreater(self.core.accepted,old)
    async def test_config_rollback_on_compositor_rejection(self):
        before=copy.deepcopy(self.core.model.config);self.core.wm.fail=True
        with self.assertRaises(Rejected):await self.core.command('configure',{'changes':{'innerGap':32,'outerGap':20}})
        self.assertEqual(self.core.model.config,before);self.assertFalse(self.core.config_path.exists())
    async def test_invalid_config_never_reaches_compositor(self):
        count=len(self.core.wm.calls)
        with self.assertRaises(Rejected):await self.core.command('configure',{'changes':{'masterRatio':-2}})
        self.assertEqual(len(self.core.wm.calls),count)
    async def test_free_canvas_does_not_assign_new_windows(self):
        await self.core.command('anchor',{})
        raw=copy.deepcopy(self.raw);raw['windows'].append({'id':2,'app_id':'other','title':'other'})
        await self.core.reconcile(raw);self.assertIsNone(self.core.model.window_zones[2])
    async def test_transient_not_tiled(self):
        raw=copy.deepcopy(self.raw);raw['windows'].append({'id':2,'app_id':'dialog','title':'dialog','transient':True})
        await self.core.reconcile(raw);self.assertNotIn(2,self.core.model.window_zones)
    async def test_focus_restores_panels(self):
        await self.core.command('pin',{});before=copy.deepcopy(self.core.model.panels)
        await self.core.command('focus',{});await self.core.command('focus',{});self.assertEqual(self.core.model.panels,before)
    async def test_config_persists_atomically(self):
        await self.core.command('configure',{'changes':{'outerGap':20}})
        self.assertEqual(json.loads(self.core.config_path.read_text())['outerGap'],20)
    async def test_fullscreen_remains_zone_member(self):
        raw=copy.deepcopy(self.raw);raw['windows']=[];raw['fullscreen']=[{'id':1,'output':'TEST-1'}]
        await self.core.reconcile(raw);self.assertIn(1,self.core.current_zone()['windows'])
    async def test_real_unix_transport_version_validation(self):
        path=self.directory/'ipc.sock'
        try:
            server=await asyncio.start_unix_server(self.core.client,str(path))
        except PermissionError:
            self.skipTest('Unix-сокеты запрещены средой; требуется повторить тест на Arch Linux')
        async with server:
            reader,writer=await asyncio.open_unix_connection(str(path))
            self.assertEqual(json.loads(await reader.readline())['type'],'State')
            writer.write(encode({'requestId':7,'schemaVersion':999,'action':'state','payload':{}}));await writer.drain()
            reply=json.loads(await reader.readline());self.assertFalse(reply['ok']);self.assertEqual(reply['requestId'],7)
            writer.write(encode({'requestId':8,'schemaVersion':1,'action':'state','payload':{}}));await writer.drain()
            reply=json.loads(await reader.readline());self.assertTrue(reply['ok']);self.assertEqual(reply['requestId'],8)
            writer.close();await writer.wait_closed()

if __name__=='__main__':unittest.main()
