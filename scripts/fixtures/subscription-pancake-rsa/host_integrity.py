#!/usr/bin/env python3
"""Compare host namespace and public configuration hashes, never modifies them."""
import hashlib,json,os,sys
from pathlib import Path
def snapshot():
 paths=['/etc/hosts','/etc/nsswitch.conf','/etc/ssl/certs/ca-certificates.crt','/etc/ca-certificates/extracted/tls-ca-bundle.pem','/etc/ssl/cert.pem']
 return {'network_namespace':os.readlink('/proc/self/ns/net'),'mount_namespace':os.readlink('/proc/self/ns/mnt'),'files':{p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in paths if Path(p).is_file()}}
if __name__=='__main__':
 state=snapshot()
 if sys.argv[1]=='record':Path(sys.argv[2]).write_text(json.dumps(state,indent=2)+'\n')
 elif sys.argv[1]=='compare':
  expected=json.loads(Path(sys.argv[2]).read_text());assert state==expected,(state,expected);print(json.dumps({'host_namespace_and_hosts_CA_unchanged':True,'state':state}))
