#!/usr/bin/env python3
import json
from pathlib import Path
root=Path(__file__).resolve().parents[1]
tokens=json.loads((root/'themes/default/tokens.json').read_text())
lines=['pragma Singleton','import QtQuick','QtObject {']
for name,value in tokens.items():
    kind='color' if isinstance(value,str) and value.startswith('#') else 'string' if isinstance(value,str) else 'int'
    lines.append(f'    readonly property {kind} {name}: {json.dumps(value)}')
lines.append('}')
(root/'shell-ui/qml/Theme.qml').write_text('\n'.join(lines)+'\n')
css=':root {\n'+''.join(f'  --ds-{k}: {v};\n' for k,v in tokens.items() if isinstance(v,str) and v.startswith('#') and len(v)==7)+'}\n'
(root/'zen-integration/tokens.css').write_text(css)
