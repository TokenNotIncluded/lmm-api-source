-- Explicit offline upgrade of the dedicated store database.
-- After installation grant exactly:
--   GRANT SELECT,INSERT,UPDATE,DELETE ON lmm_store.cart_items,
--     lmm_store.favorites TO <store runtime role>;
-- Startup never runs this migration or grants privileges.
BEGIN;
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
