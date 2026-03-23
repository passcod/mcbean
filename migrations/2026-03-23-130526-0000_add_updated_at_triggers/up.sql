CREATE OR REPLACE FUNCTION diesel_set_updated_at() RETURNS trigger AS $$
BEGIN
    IF (
        NEW IS DISTINCT FROM OLD AND
        NEW.updated_at IS NOT DISTINCT FROM OLD.updated_at
    ) THEN
        NEW.updated_at := current_timestamp;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION diesel_manage_updated_at(_tbl regclass) RETURNS VOID AS $$
BEGIN
    EXECUTE format(
        'CREATE TRIGGER set_updated_at BEFORE UPDATE ON %s FOR EACH ROW EXECUTE PROCEDURE diesel_set_updated_at()',
        _tbl
    );
END;
$$ LANGUAGE plpgsql;

SELECT diesel_manage_updated_at('users');
SELECT diesel_manage_updated_at('repositories');
SELECT diesel_manage_updated_at('specs');
SELECT diesel_manage_updated_at('proposals');
