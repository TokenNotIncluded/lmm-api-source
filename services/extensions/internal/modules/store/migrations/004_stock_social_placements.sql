-- Explicit installation-owner migration only; never run during service startup.
-- Adds per-unit stock items, product likes, traffic analytics and paid
-- placements. After applying, grant the store runtime role exactly:
--   GRANT SELECT,INSERT,UPDATE ON lmm_store.stock_items, lmm_store.product_likes,
--     lmm_store.placements TO <store runtime role>;
--   GRANT SELECT,INSERT,UPDATE,DELETE ON lmm_store.traffic_days,
--     lmm_store.traffic_receipts TO <store runtime role>;
-- The extension refuses to start until these grants match its runtime contract.
BEGIN;
-- Per-unit stock, likes, traffic and paid placements. Item contents are
-- AES-256-GCM ciphertext under the separate store stock key; never plaintext.
CREATE TABLE lmm_store.stock_items (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK (body->>'state' IN ('available','reserved','delivered','refunded','removed')),
 CHECK ((body->>'position')::bigint>0),
 CHECK (body->>'state'='removed' OR body->>'ciphertext' LIKE 'v1:%'),
 CHECK ((body->>'state' IN ('available','removed'))=(COALESCE(body->>'order_id','')=''))
);
CREATE UNIQUE INDEX store_stock_item_public_id ON lmm_store.stock_items (id);
CREATE INDEX store_stock_items_variant ON lmm_store.stock_items (shop_id,(body->>'variant_id'),(body->>'state'),((body->>'position')::bigint),id);
CREATE TABLE lmm_store.product_likes (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'user_id')::bigint>0), CHECK (jsonb_typeof(body->'liked')='boolean')
);
CREATE INDEX store_product_likes_product ON lmm_store.product_likes (shop_id,(body->>'product_id')) WHERE (body->>'liked')::boolean;
-- Traffic stores no account, address or user agent; day rows are UTC buckets.
CREATE TABLE lmm_store.traffic_days (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'day')::bigint>=0 AND (body->>'day')::bigint%86400=0),
 CHECK ((body->>'impressions')::bigint>=0), CHECK ((body->>'clicks')::bigint>=0)
);
CREATE INDEX store_traffic_days_product ON lmm_store.traffic_days ((body->>'product_id'),((body->>'day')::bigint));
CREATE INDEX store_traffic_days_day ON lmm_store.traffic_days (((body->>'day')::bigint));
CREATE TABLE lmm_store.traffic_receipts (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK (body->>'kind' IN ('impression','click'))
);
CREATE INDEX store_traffic_receipts_page ON lmm_store.traffic_receipts (((body->>'page_started_at')::bigint));
CREATE TABLE lmm_store.placements (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'months')::bigint BETWEEN 1 AND 12), CHECK ((body->>'quota')::bigint>=0),
 CHECK ((body->>'expires_at')::bigint>(body->>'created_at')::bigint)
);
CREATE INDEX store_placements_product ON lmm_store.placements (shop_id,(body->>'product_id'));
COMMIT;
