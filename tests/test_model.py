import copy
import json
import math
import tempfile
import unittest
from pathlib import Path
from drift_shell.model import Model, Rejected, Rect, defaults, validate_config, safe_area, layout, atomic_write, snap_zone, visible_slots

class ModelTests(unittest.TestCase):
    def test_clean_start_is_work(self):
        m=Model();self.assertEqual(m.config['profile'],'work');self.assertEqual(m.panels['right'],'Hidden');self.assertEqual(m.panels['dock'],'Hidden')
    def test_peek_never_changes_safe_area(self):
        for w,h in [(1672,941),(1920,1080),(800,600)]:
            self.assertEqual(safe_area(w,h,defaults(),'Hidden'),safe_area(w,h,defaults(),'Peek'))
    def test_dock_overlay_never_changes_safe_area(self):
        self.assertEqual(safe_area(1672,941,defaults(),dock='Hidden'),safe_area(1672,941,defaults(),dock='VisibleOverlay'))
    def test_pinned_reserves_exactly_once(self):
        cfg=defaults();a=safe_area(1672,941,cfg);b=safe_area(1672,941,cfg,'Pinned');self.assertEqual(a.w-b.w,302)
    def test_small_output_degrades_to_overlay(self):
        self.assertEqual(safe_area(600,400,defaults(),'Pinned'),safe_area(600,400,defaults(),'Hidden'))
    def test_layout_cells_stay_in_bounds(self):
        for w,h in [(1672,941),(800,600),(3840,2160)]:
            rect=safe_area(w,h,defaults())
            for mode in ['master','columns','rows','grid','monocle']:
                for n in range(1,60):
                    cells=layout(rect,n,mode)
                    self.assertEqual(len(cells),n)
                    for c in cells:
                        self.assertGreater(c.w,0);self.assertGreater(c.h,0)
                        self.assertGreaterEqual(c.x,rect.x);self.assertGreaterEqual(c.y,rect.y)
                        self.assertLessEqual(c.x+c.w,rect.x+rect.w);self.assertLessEqual(c.y+c.h,rect.y+rect.h)
    def test_cells_do_not_overlap_except_monocle(self):
        for mode in ['master','columns','rows','grid']:
            cells=layout(Rect(0,0,3840,2160),6,mode)
            if len(set(cells))==1:continue
            for i,a in enumerate(cells):
                for b in cells[i+1:]:self.assertTrue(a.x+a.w<=b.x or b.x+b.w<=a.x or a.y+a.h<=b.y or b.y+b.h<=a.y)
    def test_focus_restores_snapshot(self):
        m=Model();m.panels.update(right='Pinned',dock='VisibleOverlay',dnd=False);before=copy.deepcopy(m.panels)
        for _ in range(1000):
            m.toggle_focus();m.panels['right']='Peek';m.toggle_focus();self.assertEqual(m.panels,before)
    def test_profiles_preserve_zone_layout(self):
        m=Model();m.zones[0]['windows']=[2,3];before=copy.deepcopy(m.zones)
        for name in ['reference','work','minimal','showcase']:m.profile(name);self.assertEqual(m.zones,before)
    def test_invalid_config_rejected(self):
        for key,value in [('innerGap',-1),('innerGap',True),('masterRatio',float('nan')),('profile','bad'),('starCount',1001)]:
            with self.assertRaises(Rejected):validate_config({**defaults(),key:value})
    def test_unknown_fields_round_trip(self):
        self.assertEqual(validate_config({**defaults(),'future':{'a':1}})['future'],{'a':1})
    def test_atomic_write_preserves_file_on_failed_encoding(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'config.json';atomic_write(p,{'old':1})
            with self.assertRaises(ValueError):atomic_write(p,{'bad':float('nan')})
            self.assertEqual(json.loads(p.read_text()),{'old':1});self.assertEqual(p.stat().st_mode&0o777,0o600)
    def test_snap_uses_screen_space_and_zoom(self):
        zone=dict(id='one',center=[100,0],zoom=.5)
        self.assertEqual(snap_zone([zone],[0,0],.5,[1000,800]),zone)
        self.assertIsNone(snap_zone([zone],[0,0],1,[1000,800]))
        self.assertIsNone(snap_zone([zone],[10000,0],.5,[1000,800]))
    def test_slots_always_include_active(self):
        for active in range(1,10):
            s=visible_slots(active,[1,9]);self.assertIn(active,s);self.assertEqual(len(s),5);self.assertEqual(sorted(set(s)),s)
    def test_restore_drops_session_window_handles(self):
        a=Model();a.zones[0]['windows']=[11];b=Model();b.restore(a.dump());self.assertEqual(b.zones[0]['windows'],[])
    def test_bad_restore_is_transactional(self):
        a=Model();before=a.dump();bad=copy.deepcopy(before);bad['zones'][0]['name']='other';bad['tasks']=[{'done':'not-bool'}]
        with self.assertRaises(Rejected):a.restore(bad)
        self.assertEqual(a.dump(),before)
    def test_duplicate_zone_identity_rejected(self):
        m=Model();data=copy.deepcopy(m.dump());data['zones'][1]['id']=data['zones'][0]['id']
        with self.assertRaises(Rejected):m.restore(data)

if __name__=='__main__':unittest.main()
