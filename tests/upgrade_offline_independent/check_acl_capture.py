#!/usr/bin/env python3
"""Compare every saved owner/grant fact on pre-existing database objects."""
import json,sys
from pathlib import Path
root=Path(sys.argv[1]);before=json.loads((root/'independent-before.json').read_text());after=json.loads((root/'independent-after-commit.json').read_text())
a=[json.loads(x) for x in before['acl']];b=[json.loads(x) for x in after['acl']]
schemas={x[1] for x in a if x[0]=='schema-owner'}
relations={(x[1],x[2]) for x in a if x[0]=='relation-owner'}
routines={(x[1],x[2]) for x in a if x[0]=='routine-owner'}
def old(x):
 t=x[0]
 return (t.startswith('schema-') and x[1] in schemas) or (t.startswith(('relation-','column-')) and (x[1],x[2]) in relations) or (t.startswith('routine-') and (x[1],x[2]) in routines)
encode=lambda x:json.dumps(x,sort_keys=True,ensure_ascii=False)
oldfacts={encode(x) for x in a};newfacts={encode(x) for x in b if old(x)}
result={'old_schemas':len(schemas),'old_relations_including_sequences':len(relations),'old_routines':len(routines),'original_acl_and_owner_facts':len(oldfacts),'missing':sorted(oldfacts-newfacts),'added_on_existing_objects':sorted(newfacts-oldfacts),'status':'PASS' if oldfacts==newfacts else 'FAIL'}
(root/'independent-exact-old-acl.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));sys.exit(result['status']!='PASS')
