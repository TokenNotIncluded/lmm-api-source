-- Apply only to a NEW, dedicated extension database, using an installation role.
-- Give the runtime role USAGE on lmm_store and SELECT/INSERT/UPDATE on its tables.
-- Grant DELETE only on traffic_days, traffic_receipts, cart_items and favorites.
-- Never give this role credentials or grants for the Rust core database.
BEGIN;
CREATE SCHEMA lmm_store;
CREATE TABLE lmm_store.shops (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY (shop_id,id), CHECK (shop_id=id),
 CHECK (jsonb_typeof(body)='object'),
 CHECK (body->'owner'->>'kind' IN ('personal','team')),
 CHECK ((body->'owner'->>'id')::bigint>0)
);
CREATE TABLE lmm_store.products (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object')
);
CREATE TABLE lmm_store.variants (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'stock')::bigint>=0), CHECK ((body->>'reserved')::bigint>=0),
 CHECK ((body->>'claimed')::bigint>=0), CHECK ((body->>'quota')::bigint>=-1),
 CHECK ((body->'price'->>'amount_minor')::bigint>=0),
 CHECK (NOT (body->>'track_stock')::boolean OR (body->>'stock')::bigint >= (body->>'reserved')::bigint)
);
CREATE TABLE lmm_store.orders (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'quantity')::bigint>0),
 CHECK ((body->'total'->>'amount_minor')::numeric = (body->'unit_price'->>'amount_minor')::numeric * (body->>'quantity')::numeric),
 CHECK (body->>'state' IN ('awaiting_payment','payment_pending','paid','fulfilled','settlement_pending','settled','refund_pending','refunded','cancelled'))
);
CREATE TABLE lmm_store.commands (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object')
);
CREATE TABLE lmm_store.stock_movements (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object')
);
CREATE TABLE lmm_store.money_operations (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object')
);
-- Account IDs are opaque Rust references, NOT foreign keys into core tables.
CREATE INDEX store_orders_buyer ON lmm_store.orders (shop_id, (body->'buyer'->>'kind'), (body->'buyer'->>'id'),id);
CREATE INDEX store_variants_product ON lmm_store.variants (shop_id,(body->>'product_id'),id);
-- Schema v2: money units are mutually exclusive; integer credits are never
-- accepted through the external minor-unit branch.
ALTER TABLE lmm_store.variants ADD CONSTRAINT store_variant_price_unit CHECK (COALESCE((jsonb_typeof(body->'price')='object' AND (((body->'price'->>'unit')='credits' AND (body->'price'->>'amount_credits') ~ '^(0|[1-9][0-9]*)$' AND (body->'price'->>'amount_credits')::numeric<=9223372036854775807 AND ((body->'price')-ARRAY['amount_credits','unit'])='{}'::jsonb) OR ((body->'price'->>'currency') ~ '^[A-Z]{3}$' AND (body->'price'->>'amount_minor') ~ '^(0|[1-9][0-9]*)$' AND (body->'price'->>'amount_minor')::numeric<=9223372036854775807 AND ((body->'price')-ARRAY['amount_minor','currency'])='{}'::jsonb))),false));
ALTER TABLE lmm_store.orders ADD CONSTRAINT store_order_price_units CHECK (COALESCE((jsonb_typeof(body->'total')='object' AND (((body->'total'->>'unit')='credits' AND (body->'total'->>'amount_credits') ~ '^(0|[1-9][0-9]*)$' AND (body->'total'->>'amount_credits')::numeric<=9223372036854775807 AND ((body->'total')-ARRAY['amount_credits','unit'])='{}'::jsonb) OR ((body->'total'->>'currency') ~ '^[A-Z]{3}$' AND (body->'total'->>'amount_minor') ~ '^(0|[1-9][0-9]*)$' AND (body->'total'->>'amount_minor')::numeric<=9223372036854775807 AND ((body->'total')-ARRAY['amount_minor','currency'])='{}'::jsonb))),false) AND COALESCE((jsonb_typeof(body->'unit_price')='object' AND (((body->'unit_price'->>'unit')='credits' AND (body->'unit_price'->>'amount_credits') ~ '^(0|[1-9][0-9]*)$' AND (body->'unit_price'->>'amount_credits')::numeric<=9223372036854775807 AND ((body->'unit_price')-ARRAY['amount_credits','unit'])='{}'::jsonb) OR ((body->'unit_price'->>'currency') ~ '^[A-Z]{3}$' AND (body->'unit_price'->>'amount_minor') ~ '^(0|[1-9][0-9]*)$' AND (body->'unit_price'->>'amount_minor')::numeric<=9223372036854775807 AND ((body->'unit_price')-ARRAY['amount_minor','currency'])='{}'::jsonb))),false) AND CASE WHEN body->'total'->>'unit'='credits' THEN body->'unit_price'->>'unit'='credits' AND (body->'total'->>'amount_credits')::numeric=(body->'unit_price'->>'amount_credits')::numeric*(body->>'quantity')::numeric ELSE body->'total'->>'currency'=body->'unit_price'->>'currency' AND (body->'total'->>'amount_minor')::numeric=(body->'unit_price'->>'amount_minor')::numeric*(body->>'quantity')::numeric END);
-- Public resource and buyer request identities are globally unambiguous.
CREATE UNIQUE INDEX store_product_public_id ON lmm_store.products (id);
CREATE UNIQUE INDEX store_variant_public_id ON lmm_store.variants (id);
CREATE UNIQUE INDEX store_order_public_id ON lmm_store.orders (id);
-- Schema v4 (migrations/004_stock_social_placements.sql).
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
CREATE TABLE lmm_store.extore_flows (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'user_id')::bigint>0), CHECK ((body->>'expires_at')::bigint>0)
);
CREATE TABLE lmm_store.extore_connections (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK (body->>'ciphertext' LIKE 'v1:%'), CHECK ((body->>'expires_at')::bigint>0)
);
CREATE INDEX store_extore_connections_shop ON lmm_store.extore_connections (shop_id,(body->>'shop_id'));
CREATE TABLE lmm_store.extore_products (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK (char_length(COALESCE(body->>'product_id',''))>0),
 CHECK (char_length(COALESCE(body->>'source_id',''))>0)
);
CREATE UNIQUE INDEX store_extore_products_local ON lmm_store.extore_products ((body->>'product_id'));
CREATE INDEX store_extore_products_connection ON lmm_store.extore_products (shop_id,(body->>'connection_id'));
-- Schema v6 (migrations/006_account_collections.sql).
CREATE TABLE lmm_store.cart_items (
 user_id bigint NOT NULL CHECK (user_id>0),
 id text NOT NULL CHECK (id ~ '^[a-f0-9]{64}$'),
 product_id text NOT NULL CHECK (length(product_id) BETWEEN 1 AND 128),
 variant_id text NOT NULL CHECK (length(variant_id) BETWEEN 1 AND 128),
 quantity bigint NOT NULL CHECK (quantity BETWEEN 1 AND 1000),
 created_at bigint NOT NULL CHECK (created_at>0),
 updated_at bigint NOT NULL CHECK (updated_at>=created_at),
 PRIMARY KEY(user_id,id), UNIQUE(user_id,product_id,variant_id)
);
CREATE INDEX store_cart_updated ON lmm_store.cart_items (user_id,updated_at DESC,id);
CREATE TABLE lmm_store.favorites (
 user_id bigint NOT NULL CHECK (user_id>0),
 product_id text NOT NULL CHECK (length(product_id) BETWEEN 1 AND 128),
 created_at bigint NOT NULL CHECK (created_at>0),
 PRIMARY KEY(user_id,product_id)
);
CREATE INDEX store_favorites_created ON lmm_store.favorites (user_id,created_at DESC,product_id);
COMMIT;
