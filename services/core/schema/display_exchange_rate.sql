-- Display-only CNY per USD. Ledger credits and payment quotes do not read this option.
CREATE FUNCTION core_console.write_display_exchange_rate(p_value TEXT) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE result BIGINT; rate DOUBLE PRECISION;
BEGIN
    IF p_value IS NULL OR octet_length(p_value) > 32
       OR p_value !~ '^(0|[1-9][0-9]*)([.][0-9]+)?([eE][+-]?[0-9]+)?$' THEN
        RAISE EXCEPTION 'invalid display exchange rate' USING ERRCODE = '22023';
    END IF;
    rate := p_value::DOUBLE PRECISION;
    IF rate < 0.0001 OR rate > 1000000000 THEN
        RAISE EXCEPTION 'invalid display exchange rate' USING ERRCODE = '22023';
    END IF;
    PERFORM core_console.lock_public_options();
    INSERT INTO core_console.options AS o(key, value) VALUES ('USDExchangeRate', p_value)
    ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
        revision = o.revision + 1, updated_at = statement_timestamp()
    RETURNING revision INTO result;
    RETURN result;
END
$$;
REVOKE ALL ON FUNCTION core_console.write_display_exchange_rate(TEXT) FROM PUBLIC;
