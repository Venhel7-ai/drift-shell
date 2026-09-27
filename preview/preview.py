#!/usr/bin/env python3
"""Portable Qt preview. All metrics/windows here are explicitly test fixtures."""
import argparse
import json
import sys
from pathlib import Path
root=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root/'shell-core'))
from drift_shell.model import Model, SETTINGS, layout, safe_area, visible_slots, validate_config
from PySide6.QtCore import QObject, Signal, Property, Slot, QTimer, QUrl
from PySide6.QtGui import QGuiApplication
from PySide6.QtQml import QQmlApplicationEngine
from PySide6.QtQuick import QQuickWindow

class Backend(QObject):
    changed=Signal()
    def __init__(self,args):
        super().__init__()
        self.args=args
        self.model=Model()
        self.model.profile(args.profile)
        self.model.panels.update(settings=args.scene=='settings',search=args.scene=='search',notifications=args.scene=='notifications')
        if args.scene=='peek': self.model.panels['right']='Peek'
        if args.scene=='pinned': self.model.panels['right']='Pinned'
        if args.scene=='dock': self.model.panels['dock']='VisibleOverlay'
        if args.scene=='focus': self.model.toggle_focus()
        self._results=[]
    @Property('QVariantMap',notify=changed)
    def snapshot(self):
        m=self.model
        rect=safe_area(self.args.width,self.args.height,m.config,m.panels['right'],m.panels['dock'])
        cells=layout(rect,3,m.config['layout'],m.config['innerGap'],m.config['masterRatio'])
        windows=[]
        for i,(r,title,body) in enumerate(zip(cells,['Редактор · тестовое окно','Терминал · тестовое окно','Монитор · тестовое окно'],[
            'src / shell / layout.py\n\ndef safe_area(output, panels):\n    top = 44 + margin\n    right = panel.reserve\n    return output.inset(top, right)\n\n# Предпросмотр геометрии интерфейса',
            '$ drift-shell-ctl state\n\nПрофиль: '+m.config['profile']+'\nИсточник: тестовые данные\n\n$ _',
            'CPU         12 %\nПамять      41 %\nДиск        18 %\n\nДанные предпросмотра'])):
            windows.append(dict(x=r.x,y=r.y,w=r.w,h=r.h,title=title,body=body,active=i==0))
        return dict(config=m.config,settings=SETTINGS,panels=m.panels,zones=m.zones,activeZone=m.zones[0]['id'],slots=visible_slots(1),tasks=m.tasks,focus=m.focus_snapshot is not None,metrics=dict(cpu=12,memory=41,disk=18,download=1200000,upload=130000),samples=[10+(i*7)%25 for i in range(60)],notifications=[],notificationAvailable=True,compositor={'windows':[],'layout_short':'ru'},previewWindows=windows,connected=False,acceptedRevision=0,errors=['Предпросмотр: композитор не подключён'])
    @Property('QVariantList',notify=changed)
    def results(self): return self._results
    @Slot(str)
    def search(self,query):
        self._results=[dict(kind='setting',id=k,title=v['label']) for k,v in SETTINGS.items() if query.lower() in v['label'].lower()]
        self.changed.emit()
    @Slot(str,str)
    def command(self,action,encoded):
        p=json.loads(encoded);m=self.model
        if action=='configure':
            m.config=validate_config({**m.config,**p['changes']})
            if 'profile' in p['changes']:m.profile(p['changes']['profile'])
        elif action=='right':m.panels['right']='Peek' if m.panels['right']=='Hidden' else 'Hidden'
        elif action=='pin':m.panels['right']='Peek' if m.panels['right']=='Pinned' else 'Pinned'
        elif action=='dock':m.panels['dock']='VisibleOverlay' if m.panels['dock']=='Hidden' else 'Hidden'
        elif action=='focus':m.toggle_focus()
        elif action in ['settings','notifications','dnd','search-toggle']:
            key='search' if action=='search-toggle' else action;m.panels[key]=not m.panels[key]
        elif action=='dismiss':
            for k in ['search','settings','notifications']:m.panels[k]=False
            if m.panels['right']=='Peek':m.panels['right']='Hidden'
        self.changed.emit()

def main():
    p=argparse.ArgumentParser();p.add_argument('--width',type=int,default=1672);p.add_argument('--height',type=int,default=941)
    p.add_argument('--profile',default='work',choices=['work','reference','minimal','showcase','focus-presentation'])
    p.add_argument('--scene',default='desktop',choices=['desktop','peek','pinned','dock','search','focus','settings','notifications']);p.add_argument('--screenshot')
    args=p.parse_args()
    app=QGuiApplication(sys.argv);engine=QQmlApplicationEngine();backend=Backend(args)
    engine.rootContext().setContextProperty('previewBackend',backend)
    engine.rootContext().setContextProperty('previewWidth',args.width);engine.rootContext().setContextProperty('previewHeight',args.height)
    engine.load(QUrl.fromLocalFile(str(root/'preview/Preview.qml')))
    if not engine.rootObjects(): return 1
    if args.screenshot:
        def grab():
            image=engine.rootObjects()[0].grabWindow()
            app.exit(0 if image.save(args.screenshot) else 2)
        QTimer.singleShot(800,grab)
    code=app.exec()
    import shiboken6
    for obj in engine.rootObjects(): shiboken6.delete(obj)
    return code
if __name__=='__main__':sys.exit(main())
