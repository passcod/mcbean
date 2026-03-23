DROP TRIGGER set_updated_at ON users;
DROP TRIGGER set_updated_at ON repositories;
DROP TRIGGER set_updated_at ON specs;
DROP TRIGGER set_updated_at ON proposals;
DROP FUNCTION diesel_manage_updated_at;
DROP FUNCTION diesel_set_updated_at;
