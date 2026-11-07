-- Extore authorize/catalog durable state for microservice store hosts.
--   GRANT SELECT,INSERT,UPDATE ON lmm_store.extore_flows, lmm_store.extore_connections TO <runtime role>;
BEGIN;
CREATE TABLE IF NOT EXISTS lmm_store.extore_flows (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK ((body->>'user_id')::bigint>0), CHECK ((body->>'expires_at')::bigint>0)
);
CREATE TABLE IF NOT EXISTS lmm_store.extore_connections (
 shop_id text NOT NULL, id text NOT NULL, body jsonb NOT NULL,
 PRIMARY KEY(shop_id,id), CHECK(jsonb_typeof(body)='object'),
 CHECK (body->>'ciphertext' LIKE 'v1:%'), CHECK ((body->>'expires_at')::bigint>0)
);
CREATE INDEX IF NOT EXISTS store_extore_connections_shop ON lmm_store.extore_connections (shop_id,(body->>'shop_id'));
COMMIT;
