"""Minimal historical goods increment over the existing QD real-process fixture.

Synthetic owner-issued source/opening receipts only, not production migration
acceptance or a test of the complete financial_import admission workflow.
"""
import copy, hashlib, json, time

def raw(v): return json.dumps(v,ensure_ascii=False,separators=(',',':'))
def digest(v): return hashlib.sha256(raw(v).encode()).hexdigest()
def literal(v): return "'"+raw(v).replace("'","''")+"'::jsonb"
def source_hash(v): return hashlib.sha256(json.dumps(v,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()).hexdigest()

class LegacyRefundCases:
    def __init__(self, fixture, core_db, store_db, out, port, users, accounts, request, write, need, runtime):
        self.f=fixture;self.cdb=core_db;self.gdb=store_db;self.out=out;self.port=port;self.users=users;self.accounts=accounts
        self.request=request;self.write=write;self.need=need;self.runtime=runtime;self.step=0
        self.shop=digest(['legacy-web-shop-v1',{'kind':'personal','id':9}]);self.product=digest(['legacy-fixture-product'])
        self.orders=[digest(['historical-paid-fixture',n]) for n in (1,2)]
        self.history=[digest(['historical-refunded-fixture',n]) for n in (1,2)]
        self.migration='synthetic-launch-historical-balance-v1';self.archive=digest(['synthetic-source-archive']);self.mapping=digest(['synthetic-identity-map'])
        self.observed=1_700_000_100;self.grants=[];self.commands=[]

    def sql(self,q,store=False): return self.f.sql(q,self.gdb if store else self.cdb)
    def core_owner(self,q): return self.sql('SET ROLE lmm_core_owner;\n'+q)
    def seed(self):
        # Opening balances ALREADY include the old sale; never fabricate a pay,
        # capture, topup, reserve or refund. Seller spent 50 before the snapshot.
        openings=[]
        for uid,balance in [(7,1000),(9,200),(11,1000)]:
            openings.append({'migration_id':self.migration,'source_entity_key':'user:'+str(uid),'source_archive_sha256':self.archive,
                'mapping_sha256':self.mapping,'source_row_sha256':digest(['synthetic-wallet-row',uid,balance]),'observed_at':self.observed,
                'account_id':int(self.accounts[str(uid)]),'available_units':balance,'reserved_units':0,'affiliate_units':0})
        statements=['BEGIN; SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;']
        statements += ['SELECT core_billing.import_opening_balance('+literal(r)+');' for r in openings]
        manifest={'scope':'financial-opening-only-v1','fixture_only':True,'not_financial_import_acceptance':True,'opening_receipts':openings}
        statements += ["INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('"+self.migration+"','"+self.archive+"','"+self.mapping+"',"+literal(manifest)+');','COMMIT;']
        self.write(self.out/'fixture-authority.json',{'scope':'isolated synthetic owner authority, NOT actual source migration approval','manifest':manifest})
        self.core_owner('\n'.join(statements))
        go=[]
        def put(table,ident,body): go.append('INSERT INTO lmm_store.'+table+'(shop_id,id,body) VALUES(\''+self.shop+'\',\''+ident+'\','+literal(body)+');')
        put('shops',self.shop,{'id':self.shop,'owner':{'kind':'personal','id':9},'title':'Synthetic historical store'})
        put('products',self.product,{'id':self.product,'title':'Deleted historical goods','published':False,'legacy':{'status':'deleted'}})
        for n,ident in enumerate(self.orders+self.history):
            paid=n<2;buyer=7 if n%2==0 else 11;price=100 if n%2==0 else 150
            variant=digest(['historical-variant',n]);item=digest(['historical-stock',n])
            source={'id':ident,'buyer_id':buyer,'seller_id':9,'quantity':1,'unit_price_quota':price,'price_quota':price,
                'payment_method':'balance','status':'paid' if paid else 'refunded','paid_at':self.observed-100,'amount_minor':0,'fee_quota':0,'fee_held':False,'claimed_at':self.observed-50}
            projection={k:v for k,v in source.items() if k!='claimed_at'}
            order={'id':ident,'buyer':{'kind':'personal','id':buyer},'seller':{'kind':'personal','id':9},'created_by':buyer,
                'variant_id':variant,'title':'Synthetic old unit','product_title':'Deleted historical goods','quantity':1,
                'unit_price':{'unit':'credits','amount_credits':price},'total':{'unit':'credits','amount_credits':price},
                'state':'settled' if paid else 'refunded','request_hash':'legacy:'+ident,'delivered':True,
                'stock_items':[item],'delivery':'stock-items:v1:'+hashlib.sha256(item.encode()).hexdigest(),
                'legacy':{'product_id':self.product,'payment_method':'balance','paid_at':self.observed-100,'delivery_template':'card-key','pickup_code_required':True},
                'imported':{'source':'go-v0.2.118','snapshot':source_hash(source),'claimed_at':self.observed-50}}
            if not paid: order.update(refund_state='completed',legacy_refund={'id':digest(['historical-completed-refund',n]),'request_key':'original','input_hash':digest(['original-refund',n]),'state':'completed','decision':'approve','completed_at':self.observed-10})
            put('variants',variant,{'id':variant,'product_id':self.product,'title':'Synthetic card-key','enabled':False,'price':{'unit':'credits','amount_credits':price},'track_stock':True,'stock':0,'reserved':0,'claimed':1,'quota':-1,'stock_sequence':1})
            # A valid synthetic fixture AEAD envelope; no source/production key or plaintext.
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM
            import base64,os
            key=os.urandom(32);nonce=os.urandom(12);aad=('lmm-store-stock-v1\0'+self.shop+'\0'+item).encode()
            cipher='v1:'+base64.urlsafe_b64encode(nonce+AESGCM(key).encrypt(nonce,b'SYNTHETIC-UNIT',aad)).decode().rstrip('=')
            put('stock_items',item,{'id':item,'product_id':self.product,'variant_id':variant,'position':1,'state':'delivered' if paid else 'refunded','order_id':ident,'ciphertext':cipher,'created_at':self.observed-100,'updated_at':self.observed-10})
            put('orders',ident,order)
            if paid:
                sale={'id':ident+':sale','order_id':ident,'kind':'sale','from_user_id':buyer,'to_user_id':9,'quota':price}
                grant={'migration_id':self.migration,'source_archive_sha256':self.archive,'mapping_sha256':self.mapping,'observed_at':self.observed,
                    'review_proof_sha256':digest(['synthetic-owner-reviewed-goods',source,sale]),'shop_id':self.shop,'source_row_sha256':source_hash(source),
                    'source_sale_sha256':source_hash(sale),'source_order':projection,'source_sale':sale,'source_refunds':[], 'buyer_user_id':buyer,'seller_user_id':9}
                command={'key':digest(['store-legacy-refund-v1',self.shop,ident]),'kind':'legacy_refund','shop_id':self.shop,'order_id':ident,
                    'source_row_sha256':source_hash(source),'buyer':order['buyer'],'seller':order['seller'],'credits':price,'quantity':1}
                self.grants.append(grant);self.commands.append(command)
                self.core_owner('BEGIN; SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SELECT core_commerce.import_legacy_refund_grant('+literal(grant)+'); COMMIT;')
        self.sql('BEGIN;\n'+'\n'.join(go)+'\nCOMMIT;',True)
        self.write(self.out/'synthetic-grants-and-commands.json',{'grants':self.grants,'commands':self.commands,'not_actual_four_source_orders':True})
        self.initial=self.snapshot();self.write(self.out/'initial.json',self.initial)
        self.need(len(self.initial['core']['refunds'])==0 and self.initial['core']['native_orders']==0 and self.initial['store']['native_operations']==0,'fixture fabricated native payment or existing refund')
        self.balances(self.initial,1000,200,1000)

    def snapshot(self):
        core=json.loads(self.sql("SELECT jsonb_build_object('balances',(SELECT jsonb_object_agg(a.owner_account_id::text||':'||a.bucket,b.balance_units) FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id IS NOT NULL),'journals',COALESCE((SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM core_billing.ledger_journals j),'[]'::jsonb),'refunds',COALESCE((SELECT jsonb_agg(to_jsonb(o) ORDER BY order_id) FROM core_commerce.legacy_refund_operations o),'[]'::jsonb),'legacy_ledger_operations',COALESCE((SELECT jsonb_agg(to_jsonb(o) ORDER BY operation_key) FROM core_billing.ledger_operations o WHERE scope='core-legacy-store-refund'),'[]'::jsonb),'native_orders',(SELECT count(*) FROM core_commerce.orders),'unbalanced',(SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) x),'global_sum',(SELECT COALESCE(sum(balance_units),0) FROM core_billing.balance_state))::text"))
        store=json.loads(self.sql("SELECT jsonb_build_object('orders',(SELECT jsonb_object_agg(id,body) FROM lmm_store.orders),'operations',COALESCE((SELECT jsonb_agg(body ORDER BY id) FROM lmm_store.commands WHERE body->'command'->>'kind'='legacy_refund'),'[]'::jsonb),'stock',(SELECT jsonb_object_agg(id,body->>'state') FROM lmm_store.stock_items),'available',(SELECT count(*) FROM lmm_store.stock_items WHERE body->>'state'='available'),'native_operations',(SELECT count(*) FROM lmm_store.money_operations))::text",True))
        self.need(core['unbalanced']==0 and core['global_sum']==0,'real ledger entries/balance-state do not conserve units')
        self.need(core['native_orders']==0 and store['native_operations']==0,'historical refund fabricated native pay')
        return {'core':core,'store':store}
    def balances(self,s,a,seller,b):
        m=s['core']['balances']
        for uid,expected in [(7,a),(9,seller),(11,b)]:
            acct=str(self.accounts[str(uid)])
            self.need((m[acct+':wallet'],m[acct+':debt'],m[acct+':reserved'])==(expected,0,0),'original party wallet/debt/reserved mismatch')
    def call(self,label,path,body,uid,status):
        self.step+=1
        # Public Web routes use native user Bearer, never host credentials.
        # Retain routing/setup errors before parsing their possibly plain body.
        import http.client
        c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=10)
        headers={'Content-Type':'application/json'}
        if uid:headers['Authorization']='Bearer '+self.users[uid]
        try:
            c.request('GET' if body is None else 'POST',path,body=None if body is None else raw(body).encode(),headers=headers)
            response=c.getresponse();payload=response.read();code=response.status
        finally:c.close()
        name=f'legacy-http-{self.step:02d}-'+label
        self.write(self.out/(name+'.raw.json'),{'path':path,'status':code,'content_type':response.getheader('Content-Type'),'response_text':payload.decode(errors='replace')})
        r={'status':code,'body':json.loads(payload)}
        self.write(self.out/(name+'.json'),{'path':path,'request':body,'response':r})
        self.need(r['status']==status,label+': unexpected HTTP status '+str(r['status']))
        return r['body'].get('data')
    def path(self,ident):return '/api/store/orders/'+ident+'/refunds'
    def open(self,n):
        value=self.call('request-'+str(n),self.path(self.orders[n]),{'request_key':'original-full-'+str(n),'mode':'full','reason':'Synthetic original historical principal'},7 if n==0 else 11,200)
        return value['id']
    def approve(self,n,child,status):return self.call('approve-'+str(n),self.path(self.orders[n])+'/'+child+'/decision',{'decision':'approve','reason':'Original full refund'},9,status)

    def denied_sql(self,body,code,role):
        # Only the expected SQLSTATE is caught. Any unexpected error or success
        # aborts the attempt; subtransaction rolls back the rejected statement.
        self.sql('BEGIN; SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SET LOCAL ROLE '+role+'; DO $deny$ BEGIN BEGIN '+body+'; RAISE EXCEPTION \'unexpected authorization\'; EXCEPTION WHEN SQLSTATE \''+code+'\' THEN NULL; END; END $deny$; COMMIT;')
    def run(self):
        before=self.snapshot()
        self.denied_sql('SELECT core_commerce.import_legacy_refund_grant('+literal(self.grants[0])+')','42501',self.runtime)
        self.denied_sql('INSERT INTO core_commerce.legacy_refund_grants SELECT * FROM core_commerce.legacy_refund_grants WHERE false','42501',self.runtime)
        self.denied_sql('UPDATE core_commerce.legacy_refund_operations SET receipt=receipt WHERE false','42501',self.runtime)
        for field,value in [('credits',99),('source_row_sha256','0'*64),('buyer',{'kind':'personal','id':13})]:
            c=dict(self.commands[0]);c[field]=value;h=digest([c])
            self.denied_sql('SELECT core_commerce.finalize_legacy_refund('+literal(c)+",decode('"+h+"','hex'),9,"+str(self.accounts['9'])+')','P1001',self.runtime)
        c=self.commands[0]
        self.denied_sql('SELECT core_commerce.finalize_legacy_refund('+literal(c)+",decode('"+digest([c])+"','hex'),13,"+str(self.accounts['13'])+')','42501',self.runtime)
        for status in ['refunded','pending-refund']:
            grant=copy.deepcopy(self.grants[0])
            if status=='refunded':grant['source_order']['status']='refunded';code='P1000'
            else:grant['source_refunds']=[{'order_id':grant['source_order']['id'],'status':'requested'}];code='P1001'
            self.denied_sql('SELECT core_commerce.import_legacy_refund_grant('+literal(grant)+')',code,'lmm_core_owner')
        self.need(self.snapshot()==before,'negative source/authority checks changed real money or orders')
        for ident in self.history:self.call('source-refunded-read',self.path(ident),None,7 if ident==self.history[0] else 11,200)
        self.call('unauthenticated',self.path(self.orders[0]),None,None,401)
        self.call('wrong-participant',self.path(self.orders[0]),None,13,403)
        self.need(self.snapshot()==before,'historical read or rejected access created funds')
        child=self.open(0)
        ident=self.orders[0]
        self.sql("CREATE FUNCTION lmm_store.legacy_fixture_ack_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.body->>'done'='true' AND NEW.body->'command'->>'kind'='legacy_refund' AND NEW.body->'command'->>'order_id'='"+ident+"' THEN RAISE EXCEPTION 'isolated historical ACK fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER legacy_fixture_ack_fault BEFORE INSERT OR UPDATE ON lmm_store.commands FOR EACH ROW EXECUTE FUNCTION lmm_store.legacy_fixture_ack_fault();",True)
        self.approve(0,child,202);pending=self.snapshot();self.write(self.out/'lost-ack.json',pending)
        self.balances(pending,1100,100,1000)
        self.need(len(pending['core']['refunds'])==1 and len(pending['core']['legacy_ledger_operations'])==1,'real Core full refund missing or duplicated')
        self.need(pending['store']['orders'][ident]['state']=='refund_pending' and not pending['store']['operations'][0]['done'],'lost ACK forged Go completion')
        self.need(len(pending['core']['journals'])==len(before['core']['journals'])+1,'original refund did not produce exactly one journal')
        self.approve(0,child,202);self.need(self.snapshot()==pending,'original-key replay changed real financial result under ACK fault')
        self.sql('DROP TRIGGER legacy_fixture_ack_fault ON lmm_store.commands; DROP FUNCTION lmm_store.legacy_fixture_ack_fault();',True)
        deadline=time.monotonic()+20
        while True:
            recovered=self.snapshot()
            if recovered['store']['orders'][ident]['state']=='refunded':break
            self.need(time.monotonic()<deadline,'receipt-only worker failed to recover ACK');time.sleep(.25)
        self.write(self.out/'ack-recovered.json',recovered)
        self.need(recovered['core']==pending['core'],'read-only ACK recovery changed Core ledger/receipt')
        self.approve(0,child,200);self.need(self.snapshot()['core']==pending['core'],'completed replay duplicated a refund')
        # Second source order exceeds the seller's real remaining wallet by50.
        child2=self.open(1);self.approve(1,child2,202);unknown=self.snapshot();self.write(self.out/'unknown-no-core-receipt.json',unknown)
        self.need(unknown['core']==recovered['core'],'insufficient liquidity left a rejected journal or operation behind')
        self.need(unknown['store']['orders'][self.orders[1]]['state']=='refund_pending','UNKNOWN was released or falsely settled')
        self.call('unknown-get',self.path(self.orders[1]),None,11,200);time.sleep(6)
        self.need(self.snapshot()==unknown,'GET/worker executed UNKNOWN refund or changed its original identity')
        liquidity={'scope':'fixture-liquidity','operation_key':'original-seller-recovery','actor_user_id':7,'reason':'Isolated existing-funds transfer; not topup/credit or fabricated legacy pay','action':{'kind':'transfer','from_account_id':int(self.accounts['7']),'to_account_id':int(self.accounts['9']),'amount_units':50}}
        self.core_owner('BEGIN; SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SELECT core_billing.post_ledger('+literal(liquidity)+'); COMMIT;')
        funded=self.snapshot();self.balances(funded,1050,150,1000)
        self.call('funded-still-unknown-get',self.path(self.orders[1]),None,11,200)
        self.need(self.snapshot()==funded,'GET initiated refund after seller liquidity returned')
        self.approve(1,child2,200);final=self.snapshot();self.write(self.out/'final-financial-state.json',final);self.balances(final,1050,0,1150)
        self.need(len(final['core']['refunds'])==2 and len(final['core']['legacy_ledger_operations'])==2,'two original grants did not refund exactly once each')
        self.need(len(final['core']['journals'])==len(before['core']['journals'])+3,'unexpected journal beyond two refunds and one separate existing-funds transfer')
        self.need(all(v=='refunded' for v in final['store']['stock'].values()) and final['store']['available']==0,'financial refund restocked an already delivered unit')
        self.need(all(o['state']=='refunded' and not o.get('payment_ref') and not o.get('settlement_ref') for o in final['store']['orders'].values()),'historical order got synthetic native payment references')
        self.approve(1,child2,200);self.need(self.snapshot()==final,'second original-key replay changed final financial state')
        self.write(self.out/'asserted-scope.json',{'scope':'two synthetic historical-paid source orders through real Go/Core/PG, two historical refunded read-only orders','native_pay_created':False,'ack_recovery':'service-only worker','unknown':'insufficient-liquidity rollback, no receipt, GET/worker never executes, same explicit seller approval resumes','real_original_production_orders_tested':False,'financial_import_admission_tested':False,'refunds':2})
