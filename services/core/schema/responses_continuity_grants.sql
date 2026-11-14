GRANT EXECUTE ON FUNCTION
 core_model.resolve_continuation(text,text),
 core_model.record_continuation(text,bytea,text,text,text,bytea,bigint),
 core_model.activate_continuation(text),
 core_model.activate_continuations_after(integer,text) TO :"runtime_role";
